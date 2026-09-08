import * as React from 'react';
import { cn } from '../../utils';
import { Button } from './button';

/**
 * The one in-place prose block: a warning, a disclosure, an inline error, an
 * empty-store line, a restart notice.
 *
 * It replaces nine hand-rolled recipes in Settings alone, which between them
 * used four radii, three type sizes, five hand-mixed alphas and a border token
 * that does not exist (`border-borderStandard` renders only because of the
 * `@layer base` fallback in `main.css`). P4: if a surface needs a variant, the
 * variant lives in the primitive, not in a `className` at the call site — so
 * `className` here is for LAYOUT (`mt-*`, `mb-*`, `min-w-0`) and nothing else.
 *
 * Three decisions are load-bearing.
 *
 * 1. **Tone is a wash, never a coloured border.** `--wash-*` is the hue at 22%
 *    behind that same hue as ink (§2.5), and it is per-family per-mode, so the
 *    note reads correctly in Parchment, Alma Mater and Roche Limit, light and
 *    dark, with no `.dark` fork and no hardcoded rgba. `neutral` is the
 *    exception on purpose — it has no hue to wash, so it takes a real surface
 *    step plus a hairline, exactly as `badge.tsx` documents for its own
 *    neutral.
 *
 * 2. **The glyph is 16px** (`--icon-row`). A note in a settings list is a
 *    row-scale object; the 20px `--icon-banner` is for a full-bleed app-level
 *    banner, and §3.8b's rule is never two icon sizes in one cluster.
 *
 * 3. **It has a ceiling.** Past `--note-max-height` (eight lines) a notice has
 *    stopped being a notice, which is the reported defect — "banners that are
 *    essentially just a long block of text". It folds behind a fade with a
 *    "Show more" control, the way `utils/messageClamp.ts` folds a long message.
 *    {@link NoteProps.unclamped} opts out and has exactly one caller: a
 *    disclosure whose whole text is mandated (DR-17 requirement 3), where
 *    hiding half of it behind a control would defeat the requirement.
 *
 * The clamp itself is AUTHORED CSS (`.biorouter-note-clamp` in `main.css`), not
 * a Tailwind arbitrary value, for the reason `.br-swatch-ring` already records:
 * a freshly-invented arbitrary class can silently fail to generate under
 * `BIOROUTER_NO_HMR`, and a clamp that fails to generate does not degrade — it
 * prints the whole document.
 */
export type NoteTone = 'neutral' | 'info' | 'success' | 'warning' | 'danger';

const toneClass: Record<NoteTone, string> = {
  neutral: 'border border-border-subtle bg-background-muted text-text-muted',
  info: 'bg-wash-info text-text-info',
  success: 'bg-wash-success text-text-success',
  warning: 'bg-wash-warning text-text-warning',
  danger: 'bg-wash-danger text-text-danger',
};

/**
 * Where the clamp's fade terminates, per tone.
 *
 * A wash is translucent, so it cannot end a gradient: fading toward
 * `--wash-warning` paints a SECOND wash over the note's own and the bottom
 * reads a step darker; fading toward `--background-default` discards the tint
 * and reads a step lighter. `--wash-solid-*` is the same mix against the page
 * ground — the note's actual composite — and is defined beside `--wash-*` in
 * `main.css` so a family that re-points its status inks moves both together.
 *
 * Passed as an inline custom property rather than a class, because a per-tone
 * `[--note-fade-ground:…]` utility is precisely the freshly-invented arbitrary
 * class the authored rule exists to avoid depending on.
 */
const toneFadeGround: Record<NoteTone, string> = {
  neutral: 'var(--background-muted)',
  info: 'var(--wash-solid-info)',
  success: 'var(--wash-solid-success)',
  warning: 'var(--wash-solid-warning)',
  danger: 'var(--wash-solid-danger)',
};

export interface NoteProps extends Omit<React.ComponentProps<'div'>, 'role' | 'children'> {
  tone?: NoteTone;
  /** A 16px status glyph, top-aligned with the first line. */
  icon?: React.ComponentType<{ className?: string }>;
  /** One control, on the trailing edge: Retry, Dismiss, Learn more. */
  action?: React.ReactNode;
  /** `status` for a standing condition, `alert` for something that just failed. */
  role?: 'status' | 'alert';
  testId?: string;
  /**
   * Render the whole body, however long. For copy whose completeness is
   * mandated — see (3) in the component docblock. Do not reach for it to avoid
   * a "Show more" you find inelegant; that control is the point.
   */
  unclamped?: boolean;
  children?: React.ReactNode;
}

export function Note({
  tone = 'neutral',
  icon: Icon,
  action,
  role,
  testId,
  unclamped = false,
  className,
  children,
  ...props
}: NoteProps) {
  const contentRef = React.useRef<HTMLDivElement>(null);
  const [overflows, setOverflows] = React.useState(false);
  const [expanded, setExpanded] = React.useState(false);

  // Measure the CONTENT against the ceiling rather than asking a clamped box
  // whether it is clamped: once expanded, a clamped box reports no overflow and
  // the control that expanded it would vanish, re-clamping the note.
  React.useLayoutEffect(() => {
    const element = contentRef.current;
    if (!element || unclamped) {
      setOverflows(false);
      return;
    }
    const measure = () => {
      const limit = Number.parseFloat(
        getComputedStyle(element).getPropertyValue('--note-max-height')
      );
      // jsdom has no layout engine and resolves no custom properties, so this
      // is `NaN` there and the control never renders in a render test. That is
      // correct rather than a workaround: the clamp is a layout behaviour and
      // is asserted at the source (`styles/noteClamp.test.ts`).
      setOverflows(Number.isFinite(limit) && element.scrollHeight > limit + 1);
    };
    measure();
    if (typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, [unclamped, children]);

  const clamped = !unclamped && overflows && !expanded;

  return (
    <div
      role={role}
      data-testid={testId}
      style={{ ['--note-fade-ground' as string]: toneFadeGround[tone] }}
      className={cn(
        'flex items-start gap-2 rounded-element px-3 py-2.5 text-supporting',
        toneClass[tone],
        className
      )}
      {...props}
    >
      {Icon && <Icon className="mt-0.5 h-4 w-4 shrink-0" aria-hidden />}
      <div className="min-w-0 flex-1 [overflow-wrap:anywhere]">
        <div ref={contentRef} className={clamped ? 'biorouter-note-clamp' : undefined}>
          {children}
        </div>
        {!unclamped && overflows && (
          <Button
            type="button"
            variant="link"
            className="mt-1 h-auto p-0 text-supporting font-normal"
            aria-expanded={expanded}
            onClick={() => setExpanded((open) => !open)}
          >
            {expanded ? 'Show less' : 'Show more'}
          </Button>
        )}
      </div>
      {action ? <div className="flex shrink-0 items-center">{action}</div> : null}
    </div>
  );
}

export default Note;
