/**
 * react-toastify's own colours, re-pointed at Biorouter's tokens (Red is rare, design.md 9.14).
 *
 * The library declares its palette on `:root` in the vendored `ReactToastify.css`: a red error
 * (`hsl(6, 78%, 57%)`, rgb 231 77 60), a green success, a rainbow progress gradient. Biorouter
 * draws its own card (`NotificationSurface`) and turns the library's icon and progress bar off,
 * but the library still renders the progress bar on every toast (hidden, opacity 0) and paints
 * it from these variables, so an error toast carried the library's red under the card (Phase C,
 * T8). Every colour the library can paint is therefore named here, as a token.
 *
 * ⚠ They are set on the container, inline, not on `:root` in `main.css`: the vendored sheet is
 * imported after `main.css`, so a `:root` rule of ours loses on source order, the same reason
 * `--toastify-z-index` is set there (App.tsx). And each one is set directly: the library's
 * `--toastify-color-progress-error: var(--toastify-color-error)` is resolved on `:root`, so
 * re-pointing only `--toastify-color-error` below it would not reach the progress bar.
 *
 * Progress bars and the spinner are neutral ink; the status colours follow the app's own status
 * tokens, whose danger tone is already ink. `toastLibraryColors.browser.test.ts` checks that every
 * colour the vendored sheet declares is covered and that the container's value wins.
 */
export const TOAST_LIBRARY_COLORS = {
  '--toastify-color-light': 'var(--background-default)',
  '--toastify-color-dark': 'var(--background-default)',
  '--toastify-color-info': 'var(--text-info)',
  '--toastify-color-success': 'var(--text-success)',
  '--toastify-color-warning': 'var(--text-warning)',
  '--toastify-color-error': 'var(--text-danger)',
  '--toastify-color-transparent': 'var(--overlay-hover)',
  '--toastify-icon-color-info': 'var(--text-info)',
  '--toastify-icon-color-success': 'var(--text-success)',
  '--toastify-icon-color-warning': 'var(--text-warning)',
  '--toastify-icon-color-error': 'var(--text-danger)',
  '--toastify-text-color-light': 'var(--text-default)',
  '--toastify-text-color-dark': 'var(--text-default)',
  '--toastify-text-color-info': 'var(--text-default)',
  '--toastify-text-color-success': 'var(--text-default)',
  '--toastify-text-color-warning': 'var(--text-default)',
  '--toastify-text-color-error': 'var(--text-default)',
  '--toastify-color-progress-light': 'var(--text-muted)',
  '--toastify-color-progress-dark': 'var(--text-muted)',
  '--toastify-color-progress-info': 'var(--text-muted)',
  '--toastify-color-progress-success': 'var(--text-muted)',
  '--toastify-color-progress-warning': 'var(--text-muted)',
  '--toastify-color-progress-error': 'var(--text-muted)',
  '--toastify-spinner-color': 'var(--text-muted)',
  '--toastify-spinner-color-empty-area': 'var(--background-fill)',
} as const;
