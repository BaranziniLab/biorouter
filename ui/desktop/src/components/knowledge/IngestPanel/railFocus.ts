/**
 * Where keyboard focus goes when the control that held it leaves the Sources rail (V3-R6-1).
 *
 * Staging a paste, cancelling the box, removing a staged row, clearing a list, dismissing a warning
 * and retrying a failed digest each UNMOUNT the control the person just pressed. Chromium hands a
 * removed focused element's focus to `<body>`, so a keyboard user lost their place in the rail
 * every time and had to Tab in from the top. (jsdom does the same, which is why the cases in
 * `IngestPanel.test.tsx` can pin it.) The rule is one sentence: focus goes to the thing the press
 * produced or, where it produced nothing, to the nearest thing that is still there.
 *
 * The panel names a target when it handles the press and `railFocusElement` finds it in the commit
 * that follows, once the change is on screen. Targets are DOM facts (`data-*` on the rows), not
 * React refs, because the rows, the dismiss buttons and the summary strip belong to three
 * components that must not know about each other.
 */
export type RailFocusTarget =
  /** "Paste text" in the drop zone: the rail's first control, and what a box or a list returns to. */
  | { to: 'paste' }
  /** The Digest / Stop button, which a retry turns from one into the other. */
  | { to: 'digest' }
  /** A staged row itself (it takes programmatic focus, never a tab stop). */
  | { to: 'staged-row'; id: string }
  /** A staged row's Remove button. */
  | { to: 'staged-remove'; id: string }
  /** A warning's Dismiss button. */
  | { to: 'warning-dismiss'; id: string };

const PASTE = '[data-testid="knowledge-ingest-paste-text"]';
const DIGEST = '[data-testid="knowledge-digest-button"]';
const STAGED_ROW = '[data-staged-id]';
const WARNING_DISMISS = '[data-warning-id]';

/** An element by the id a `data-*` attribute carries. Compared, not interpolated: ids are free text. */
function byData(rail: HTMLElement, selector: string, attribute: string, id: string) {
  return (
    Array.from(rail.querySelectorAll<HTMLElement>(selector)).find(
      (el) => el.getAttribute(attribute) === id
    ) ?? null
  );
}

/** The element a target names, or null when it is not on screen. */
export function railFocusElement(rail: HTMLElement, target: RailFocusTarget): HTMLElement | null {
  switch (target.to) {
    case 'paste':
      return rail.querySelector<HTMLElement>(PASTE);
    case 'digest':
      return rail.querySelector<HTMLElement>(DIGEST);
    case 'staged-row':
      return byData(rail, STAGED_ROW, 'data-staged-id', target.id);
    case 'staged-remove':
      return (
        byData(rail, STAGED_ROW, 'data-staged-id', target.id)?.querySelector<HTMLElement>(
          'button'
        ) ?? null
      );
    case 'warning-dismiss':
      return byData(rail, WARNING_DISMISS, 'data-warning-id', target.id);
  }
}

/**
 * The row to land on after `id` goes: the one that takes its place (the next), else the one above
 * it (the previous), else null. The same rule serves the staged list and the warnings.
 */
export function neighbourOf<T extends { id: string }>(list: readonly T[], id: string): T | null {
  const at = list.findIndex((entry) => entry.id === id);
  if (at < 0) return null;
  return list[at + 1] ?? list[at - 1] ?? null;
}
