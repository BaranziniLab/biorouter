import { cn } from '../../utils';

/**
 * The chat composer card's classes, in one place so `ChatInput` and the browser
 * test that paints them under the real stylesheet (`composerFill.browser.test.tsx`)
 * can never disagree.
 *
 * Crew's composer shares `.biorouter-composer-card` (the focus and working edges
 * in `main.css`) and keeps its own outlined card: Crew is the reference and is
 * not converted (R-27).
 */
export function composerCardClassName({ isDraggingOver }: { isDraggingOver: boolean }): string {
  return cn(
    'relative flex min-w-0 flex-col rounded-container py-2.5 pr-3 pl-4',
    // FILLS BEFORE LINES (R-27, design.md 9.11): the card is the subtle fill
    // with no resting outline. The 1px border stays, transparent, so the focus
    // edge and the working edge ride on it without moving a pixel, and forced
    // colours draw it in the system ink.
    'bg-background-fill border border-transparent',
    // A fill never sits on a fill: the quiet Send, the queue's edit field and a
    // thumbnail's remove control step back to the page ground inside the card
    // (`.br-fill-surface`, the primitives block of `main.css`).
    'br-fill-surface',
    // Its state changes (the focus edge, drag-over) on the motion tokens,
    // authored in `composer.css`.
    'br-composer-card',
    // The focus edge (`:has(textarea:focus)`, full accent) and the working edge
    // (`[data-working]`) are authored CSS in `main.css`, shared with Crew's
    // composer.
    'biorouter-composer-card',
    // Drag-over is a transient state worth showing: an inset accent edge, so it
    // costs no layout, and the fill steps to the strong step as a hover would.
    isDraggingOver && 'inset-shadow-accent bg-background-fill-strong'
  );
}
