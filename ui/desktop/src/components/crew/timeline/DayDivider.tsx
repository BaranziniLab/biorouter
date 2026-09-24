/**
 * A day's hairline with its pill: Today, Yesterday, "Monday, September 22", or
 * "September 22, 2025".
 *
 * ⚠ Two siblings, not one box, and both must be direct children of the day's
 * `<section className="crew-day">`: a sticky element sticks only inside its
 * parent, so the pill rides at the top of the log for exactly as long as its
 * day is on screen and then hands over to the next day's pill. Wrapped in a box
 * of its own it would never stick. The hairline stays in the flow.
 */
export function DayDivider({ label }: { label: string }) {
  return (
    <>
      <div className="crew-day-rule" aria-hidden="true" />
      <div className="crew-day-label" role="separator" aria-label={label}>
        <span aria-hidden="true" className="crew-day-pill text-supporting text-text-muted">
          {label}
        </span>
      </div>
    </>
  );
}
