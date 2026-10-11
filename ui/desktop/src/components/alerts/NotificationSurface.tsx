import * as React from 'react';
import { cn } from '../../utils';
import { CheckCircle, AlertCircle, AlertTriangle, Info, X } from '../icons/app-icons';
import { Spinner } from '../ui/spinner';

/**
 * Shared notification / alert layout primitive (design.md §4.2–§4.3, Direction B).
 *
 * ONE component owns the layout that both the transient toast and the inline
 * banner render through, so the two long-standing defects are fixed at the
 * source instead of per-call-site:
 *   1. the status icon lives in a fixed leading column, `self-start`, so it
 *      anchors to the title's first line at ANY height (never floats to the
 *      vertical midpoint of a tall toast);
 *   2. the close-button gutter is reserved ONCE on the container — callers add
 *      no `pr-*`, so text/actions can never slide under the ×.
 * Status is expressed only by the tinted icon chip; the surface stays neutral
 * ("colour is evidence; surfaces stay neutral").
 *
 * ONE GEOMETRY, TWO DENSITIES. A toast is sometimes title+body and sometimes
 * title-only, and both are the same object: `items-start` + a `self-start` chip mean
 * the two densities share a top edge and a first-line centre, and the two-line form
 * simply grows downward. Nothing re-centres or jumps between them. The numbers:
 *
 *   toast (py-2.5 = 10px):  chip centre 10 + 28/2 = 24px; close (main.css) top 14 + 20/2 = 24px
 *   title-only toast height: 10 + 28 + 10 = 48px — a tidy bar
 *   banner (p-3 = 12px):    chip centre 12 + 14 = 26px; close top-4 16 + 10 = 26px;
 *                           first text line 12 + 5 + 18/2 = 26px
 *
 * DEVIATIONS FROM design.md, both deliberate and approved:
 *   · Radius is `rounded-container` (12px), not §4.3's `--radius-lg` (8px). 12px is what
 *     every other floating surface uses (popover, dropdown, select menu), and a
 *     notification is a floating surface — matching them beats matching the table.
 *   · §4.3 asks for a 3px left status bar. We keep the tinted icon chip instead: it
 *     reads at a glance, survives both themes, and doesn't paint a coloured stripe
 *     down an otherwise calm neutral surface. Code wins; the spec yields. Do NOT
 *     add the bar.
 */
export type NotificationStatus = 'success' | 'error' | 'warning' | 'info' | 'loading';

// `pr-12` reserves the close gutter ONCE (react-toastify always renders its ×, whose
// 20px/right-2.5/top-14px geometry lives in main.css `.Toastify__close-button`), so no
// caller adds padding and no content can slide under the ×. See §4.2's compact rule.
// A toast floats, so it is the one floating-surface recipe (`.biorouter-popover-surface`):
// the hairline and `--shadow-popover`, and in dark the lifted ground and firmer edge every
// menu takes (R-27 keeps lines on floating surfaces; V1-M5 lifted them in dark). A
// "View details" button inside rests as the popover's quiet well.
export const TOAST_SURFACE_CLASS_NAME = `relative mb-3 pl-3 pr-12 py-2.5 rounded-container w-full min-w-0
  flex items-start overflow-hidden cursor-pointer
  text-text-default biorouter-popover-surface bg-background-default`;

// `loading` draws the one Spinner (spec 2.6) instead of a static glyph.
const STATUS_ICON: Record<
  Exclude<NotificationStatus, 'loading'>,
  React.FC<{ className?: string }>
> = {
  success: CheckCircle,
  error: AlertCircle,
  warning: AlertTriangle,
  info: Info,
};

// Tailwind's JIT only sees class strings that appear literally in source, so the
// per-status tints are written out in full (no interpolation). The chip fill is
// the status hue at a low alpha over the neutral surface; dark mode needs a touch
// more to read against the deeper ground.
const CHIP_TINT: Record<NotificationStatus, string> = {
  success: 'bg-background-success/10 dark:bg-background-success/20 text-text-success',
  error: 'bg-background-danger/10 dark:bg-background-danger/20 text-text-danger',
  warning: 'bg-background-warning/10 dark:bg-background-warning/20 text-text-warning',
  info: 'bg-background-info/10 dark:bg-background-info/20 text-text-info',
  // The neutral counterpart of the tints above: ink at 10%, so it reads on the
  // toast, on a lifted dark toast and on a filled banner alike (a fixed
  // `--background-medium` equalled the lifted dark ground).
  loading: 'bg-overlay-pressed text-text-muted',
};

export interface NotificationContentProps {
  status: NotificationStatus;
  title?: React.ReactNode;
  message?: React.ReactNode;
  /** Action-button row, rendered beneath the message. */
  actions?: React.ReactNode;
  /** Expanded region (e.g. the grouped-extension detail list). */
  children?: React.ReactNode;
  /** Override the default status glyph. */
  icon?: React.ReactNode;
  /**
   * Toast tier only: three wrapped lines is the hard ceiling (§3.7) — a fourth
   * is what promotes a notice to a banner or to the detail dialog. Inline
   * banners are not clamped, because there the full text IS the content.
   */
  clampMessage?: boolean;
  className?: string;
}

const hasText = (v: React.ReactNode): boolean => v != null && v !== '';

/**
 * The chip + text block. Used directly as the body of a react-toastify toast
 * (where the toast wrapper owns the surface + the reserved close gutter) and
 * wrapped by {@link NotificationSurface} for inline banners.
 */
export function NotificationContent({
  status,
  title,
  message,
  actions,
  children,
  icon,
  clampMessage = false,
  className,
}: NotificationContentProps) {
  const glyph =
    icon ??
    (status === 'loading' ? (
      <Spinner />
    ) : (
      React.createElement(STATUS_ICON[status], { className: 'w-4 h-4' })
    ));
  return (
    <div className={cn('flex gap-3', className)}>
      <span
        data-status={status}
        aria-hidden="true"
        className={cn(
          'flex-none self-start flex items-center justify-center w-7 h-7 rounded-element',
          CHIP_TINT[status]
        )}
      >
        {glyph}
      </span>
      <div data-notification-text className="flex-1 min-w-0 pt-[5px]">
        {hasText(title) && (
          <div className="text-secondary font-semibold text-text-default [overflow-wrap:anywhere]">
            {title}
          </div>
        )}
        {hasText(message) && (
          // With a title the message is secondary (muted); on its own it IS the
          // content and reads at full strength.
          <div
            // A clamped message keeps its full text reachable on hover; the
            // durable copy of an error is the detail dialog or "Copy error".
            title={clampMessage && typeof message === 'string' ? message : undefined}
            className={cn(
              'text-secondary [overflow-wrap:anywhere]',
              hasText(title) ? 'mt-0.5 text-text-muted' : 'text-text-default',
              clampMessage && 'line-clamp-3'
            )}
          >
            {message}
          </div>
        )}
        {actions && <div className="mt-2.5 flex flex-wrap gap-2">{actions}</div>}
        {children}
      </div>
    </div>
  );
}

export interface NotificationSurfaceProps extends NotificationContentProps {
  /** Render a dismiss control; the gutter is reserved automatically. */
  onClose?: () => void;
  /** Toasts float (elevation); inline banners are flat (default). */
  elevated?: boolean;
  role?: string;
  dismissLabel?: string;
}

/**
 * Inline banner shell: the neutral card around {@link NotificationContent}, with
 * an optional dismiss control that lives inside the once-reserved gutter.
 *
 * In place it is a subtle fill with no outline (R-27, a note): `--background-fill`
 * with the 1px border kept TRANSPARENT, so forced colours still draw its edge
 * and nothing moves, and `.br-fill-surface`, so a button inside rests on the
 * page ground. Elevated it floats, and takes the floating-surface recipe
 * (hairline and shadow, lifted in dark) on the page ground instead.
 */
export function NotificationSurface({
  onClose,
  elevated = false,
  role,
  dismissLabel = 'Dismiss',
  className,
  ...content
}: NotificationSurfaceProps) {
  return (
    <div
      data-testid="notification-surface"
      role={role}
      className={cn(
        'relative rounded-container p-3',
        elevated
          ? 'biorouter-popover-surface shadow-popover bg-background-default'
          : 'br-fill-surface border border-transparent bg-background-fill',
        onClose && 'pr-10',
        className
      )}
    >
      <NotificationContent {...content} />
      {onClose && (
        <button
          type="button"
          onClick={onClose}
          aria-label={dismissLabel}
          // §4.2 compact close: 20px ghost, rounded-sm, right-2.5, 14px icon,
          // optically centred on the FIRST LINE — never on a tall banner's midpoint.
          // `top-4` is that centring, not a magic number: the container's p-3 (12px)
          // plus half the 28px chip puts the first line's centre at 26px, and a 20px
          // button centres there at top = 26 − 10 = 16px = top-4. It is the same
          // control as the toast's ×, which sits at top-14px only because the toast
          // pads 10px instead of 12px. Change the container padding and this must
          // move with it — `dismiss control shares the chip's centre line` in the
          // tests is what catches it.
          className="absolute right-2.5 top-4 inline-flex h-5 w-5 items-center justify-center rounded-inner text-text-subtle transition-colors hover:bg-overlay-hover hover:text-text-default"
        >
          <X className="h-3.5 w-3.5" />
        </button>
      )}
    </div>
  );
}
