import { timelineCopy } from './copy';

/**
 * A day's marker: Today, Yesterday, "Monday, September 22", or "September 22,
 * 2025", on a full-width band that rides the top of the log while its day is
 * on screen and then hands over to the next day's.
 *
 * ⚠ The band must be a direct child of the day's `<section className="crew-day">`:
 * a sticky element sticks only inside its parent, so wrapped in a box of its own
 * it would never stick.
 *
 * A band, not a pill (Q3-19). A pill riding over the text covered the middle of
 * whatever line was under it — "colum[Today]ies" — and in forced colours lost
 * its box and sat on the words. The band spans the whole column on the page's
 * ground with a hairline under it, so a line scrolling beneath it is hidden
 * whole, never cut mid-word.
 *
 * `newOnRule`: the day's first message is also the first unread one, so every
 * message of the day below it is new. Instead of a second rule under this one
 * (the New line), the band's own hairline turns the accent and carries "New" at
 * its end (Q2-53). That stays true while the band rides the top: it rides only
 * over this day, and all of this day is new. For assistive technology the "New
 * messages" separator is its own element, in reading order just before the day,
 * where the New line would have been.
 */
export function DayDivider({ label, newOnRule = false }: { label: string; newOnRule?: boolean }) {
  return (
    <>
      {newOnRule && (
        <div
          className="crew-day-new-separator sr-only"
          role="separator"
          aria-label={timelineCopy.newLineLabel}
        />
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
        {newOnRule && (
          <span aria-hidden="true" className="crew-day-new text-chip text-text-accent">
            {timelineCopy.newLine}
          </span>
        )}
      </div>
    </>
  );
}
