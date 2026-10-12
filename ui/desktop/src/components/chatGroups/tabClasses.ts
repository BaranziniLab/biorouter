/**
 * Class strings shared by the document tab strips (the chat strip and the
 * terminal dock), kept free of React so a Chromium test can paint exactly what
 * the strips render (`tabFill.browser.test.ts`).
 */

/**
 * A tab's close control: a 20px ghost, shown on hover or keyboard focus and never
 * resting on every tab (vision checklist 8). Its hover is the 5% ink overlay
 * rather than a fill token, because it sits on whatever the tab under it paints
 * (nothing, the hover overlay or the selected fill) and a fixed fill vanishes on
 * one of them: the old `--background-medium` equalled the selected fill in dark
 * (R-27). The preview panel's tabs draw the same control.
 */
export const TAB_CLOSE_CLASS =
  'flex-none rounded-inner p-0.5 text-text-muted opacity-0 transition-[background-color,color,opacity] hover:bg-overlay-hover hover:text-text-default focus-visible:opacity-100 group-hover:opacity-100';

/**
 * The "+" after the last tab (a new chat, a new terminal): a 24px ghost square
 * that hovers on the band's fill step, as the band's pressed toggles do (R-27).
 */
export const TAB_NEW_CLASS =
  'br-tab-new ml-0.5 flex h-6 w-6 flex-none items-center justify-center rounded-element text-text-muted transition-colors hover:bg-background-fill-strong hover:text-text-default';
