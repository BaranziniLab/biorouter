import { useState, type ReactNode } from 'react';
import { ChevronRight } from '../icons/app-icons';
import './summaryAgents.css';

/**
 * A summary section's open state (spec 0.3): the default applies only until the
 * person toggles the section in this chat; after that their choice holds for as
 * long as the summary stays mounted. `open = choice ?? defaultOpen`.
 */
export function useSectionToggle(defaultOpen: boolean): [boolean, (open: boolean) => void] {
  const [choice, setChoice] = useState<boolean | null>(null);
  return [choice ?? defaultOpen, setChoice];
}

/**
 * The one collapsible heading of the Chat summary (To do, Decisions), drawn like
 * the roster's group toggle (`.br-agents__group-toggle`): the label, a muted
 * count, and a chevron on the right that turns 90° when open, 28px, no fill at
 * rest and the card's hover step under the pointer. The toggle is a button inside
 * the `h3`, so the heading stays a heading; `trailing` (the Monitors button) sits
 * after it on the same line. The live count is the caller's, a sibling, as before.
 */
export function SummarySectionHeading({
  label,
  count,
  open,
  onOpenChange,
  controls,
  trailing,
  children,
  testId,
}: {
  label: string;
  /** The visible count, beside the label; read by the caller's live region. */
  count?: ReactNode;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** The id of the content the toggle shows and hides. */
  controls: string;
  trailing?: ReactNode;
  /** The caller's `sr-only` live count, kept a sibling of the heading's `h3`. */
  children?: ReactNode;
  testId?: string;
}) {
  return (
    <div
      className="br-summary__heading"
      data-collapsible=""
      data-trailing={trailing ? '' : undefined}
    >
      <h3 className="br-summary__label">
        <button
          type="button"
          className="br-summary__toggle"
          aria-expanded={open}
          aria-controls={open ? controls : undefined}
          data-testid={testId}
          onClick={() => onOpenChange(!open)}
        >
          <span className="br-summary__toggle-label">{label}</span>
          {count !== undefined && count !== null && count !== '' ? (
            <span className="br-summary__count" aria-hidden="true">
              {count}
            </span>
          ) : null}
          <span className="br-agents__chevron" data-open={open ? '' : undefined} aria-hidden="true">
            <ChevronRight />
          </span>
        </button>
      </h3>
      {trailing}
      {children}
    </div>
  );
}
