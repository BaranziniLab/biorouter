import { timelineCopy } from './copy';

/**
 * A day's hairline with its pill: Today, Yesterday, "Monday, September 22", or
 * "September 22, 2025".
 *
 * ⚠ Two siblings, not one box, and both must be direct children of the day's
 * `<section className="crew-day">`: a sticky element sticks only inside its
 * parent, so the pill rides at the top of the log for exactly as long as its
 * day is on screen and then hands over to the next day's pill. Wrapped in a box
 * of its own it would never stick. The hairline stays in the flow. The pill
 * wears a ring of the canvas, so while it rides over a line of text the words
 * stop short of it instead of running into its edge (Q2-53).
 *
 * `newOnRule`: the day's first message is also the first unread one. Instead of
 * a second rule 30px under this one (the New line), this rule itself turns the
 * accent and carries "New" at its end, and is the "New messages" separator
 * (Q2-53). It is never sticky: only the day's pill rides the top.
 */
export function DayDivider({ label, newOnRule = false }: { label: string; newOnRule?: boolean }) {
  return (
    <>
      {newOnRule ? (
        <div
          className="crew-day-rule"
          data-new="true"
          role="separator"
          aria-label={timelineCopy.newLineLabel}
        >
          <span aria-hidden="true" className="crew-day-rule-new text-chip text-text-accent">
            {timelineCopy.newLine}
          </span>
        </div>
      ) : (
        <div className="crew-day-rule" aria-hidden="true" />
      )}
      <div
        className="crew-day-label"
        role="separator"
        aria-label={label}
        data-new={newOnRule ? 'true' : undefined}
      >
        <span aria-hidden="true" className="crew-day-pill text-supporting text-text-muted">
          {label}
        </span>
      </div>
    </>
  );
}
