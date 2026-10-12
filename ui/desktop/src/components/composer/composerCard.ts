import { cn } from '../../utils';

/**
 * The composer card's SURFACE: its ground, its edge and the hooks its state
 * edges key off. Every composer card in the chat draws it, so they can never
 * disagree: `ChatInput`'s (chat and Home, through `composerCardClassName`
 * below) and the in-place message editor's (`UserMessage`, ruling 21: editing a
 * message is writing in a composer). Each card adds its own layout.
 *
 * The card is the one box the fill rule (R-27) does not reach (owner message 17,
 * R-32): it is the page ground with a resting hairline, exactly the class pair
 * Crew's composer writes, so the composers draw one card.
 */
export const composerCardSurfaceClassName = cn(
  // THE PAGE GROUND AND ONE HAIRLINE (R-32): white in light, the page
  // ground in dark, with the 1px edge at 60% of `--border-subtle`, the value
  // the card had before R-27. The focus edge and the working edge turn this
  // same border the accent; nothing else about the card draws a line.
  //
  // No `.br-fill-surface`: the card is not a filled box, so what sits in it
  // (the quiet Send, the queue's edit field, a thumbnail's remove control, the
  // editor's secondary button) rests on the fill as it does on any page
  // ground, and hovers a step past it.
  'bg-background-default border border-border-subtle/60',
  // Its state changes (the focus edge, drag-over) on the motion tokens,
  // authored in `composer.css`.
  'br-composer-card',
  // The focus edge (`:has(textarea:focus)`, full accent) and the working edge
  // (`[data-working]`) are authored CSS in `main.css`, shared with Crew's
  // composer.
  'biorouter-composer-card'
);

/**
 * The chat composer card's classes, in one place so `ChatInput` and the browser
 * test that paints them under the real stylesheet (`composerCard.browser.test.tsx`)
 * can never disagree. The Home composer is the same `ChatInput`, so this is its
 * card too.
 */
export function composerCardClassName({ isDraggingOver }: { isDraggingOver: boolean }): string {
  return cn(
    'relative flex min-w-0 flex-col rounded-container py-2.5 pr-3 pl-4',
    composerCardSurfaceClassName,
    // Drag-over is a transient state worth showing: an inset accent edge, so it
    // costs no layout, and the strong fill under it.
    isDraggingOver && 'inset-shadow-accent bg-background-fill-strong'
  );
}
