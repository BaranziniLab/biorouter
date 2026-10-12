import * as React from 'react';
import { Skeleton } from '../ui/skeleton';
import { Tooltip, TooltipContent, TooltipTrigger } from '../ui/Tooltip';
import './schedule.css';

/**
 * The detail view's facts as ONE FILLED CARD (owner message 13, R-27): a standalone object
 * drawn as the subtle fill with a transparent edge and no outline, the way the summary rail's
 * card is. The look is authored in `schedule.css` (`.br-schedule-facts`); the class
 * `br-fill-surface` is what makes a control inside it (the running schedule's "Open chat")
 * rest on the page ground, because a fill never sits on a fill.
 */
export function ScheduleFacts({ children }: { children: React.ReactNode }) {
  return <div className="br-schedule-facts br-fill-surface">{children}</div>;
}

/**
 * One row of the facts card: the label on the left, the fact on its trailing edge (astryx
 * §4.5's definition row).
 *
 * A fact is not a control, so it is `.br-schedule-fact` (`schedule.css`), not the shared
 * `.biorouter-settings-row`: that recipe paints a hover wash and a focus edge, and the facts do
 * nothing when pressed (V3-14).
 */
export function FactRow({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="br-schedule-fact">
      <span className="text-body text-text-muted">{label}</span>
      {children}
    </div>
  );
}

/**
 * A fact: names, never ids (principle 10). A path or a raw cron is one hover away in `tooltip`,
 * never on the row.
 */
export function DefinitionRow({
  label,
  children,
  tone,
  tooltip,
}: {
  label: string;
  children: React.ReactNode;
  tone?: 'danger';
  tooltip?: React.ReactNode;
}) {
  const value = (
    <span
      className={[
        'min-w-0 text-body tabular-nums [overflow-wrap:anywhere]',
        tone === 'danger' ? 'text-text-danger' : 'text-text-default',
      ].join(' ')}
    >
      {children}
    </span>
  );
  return (
    <FactRow label={label}>
      {tooltip ? (
        <Tooltip>
          <TooltipTrigger asChild>{value}</TooltipTrigger>
          <TooltipContent>{tooltip}</TooltipContent>
        </Tooltip>
      ) : (
        value
      )}
    </FactRow>
  );
}

/**
 * The card while the schedule loads. The bars rest on the strong fill: the primitive's own
 * ground is `--background-muted`, which IS `--background-fill` in both modes, so on this card
 * they would be invisible.
 */
export function ScheduleFactsSkeleton() {
  return (
    <div className="br-schedule-facts br-fill-surface" aria-hidden>
      <FactRowSkeleton start="w-24" end="w-40" />
      <FactRowSkeleton start="w-24" end="w-56" />
    </div>
  );
}

function FactRowSkeleton({ start, end }: { start: string; end: string }) {
  return (
    <div className="br-schedule-fact">
      <Skeleton className={`h-4 bg-background-fill-strong ${start}`} />
      <Skeleton className={`h-4 bg-background-fill-strong ${end}`} />
    </div>
  );
}
