import * as React from 'react';
import { Slot } from '@radix-ui/react-slot';
import { cn } from '../../utils';

/**
 * The one small-label primitive, in the two tiers §3.4 makes explicit.
 *
 * The distinction is not decorative and it is enforced by the type, because the
 * app previously had four hand-rolled recipes and no way to say which was which:
 *
 * - **badge** (default) — 20px. Carries STATUS: what a thing currently is.
 * - **chip** — 24px. Carries a CATEGORY or a filter: what a thing is filed under.
 *
 * A chip is the larger of the two because it is the one you may eventually be
 * able to act on (§3.4 allows it a 16px remove control); a badge is a read-only
 * fact and gets out of the way. Both are squared at `--radius-inner`: A-04
 * restricts `--radius-full` to status dots, the switch knob and avatars, so the
 * two tiers are separated by height and role rather than by corner — which also
 * means neither can quietly become the other by losing a class.
 *
 * Tone is one formula, not six hand-picked pairs: the hue at 22% behind that same
 * hue's saturated ink (§2.5). `neutral` is the exception on purpose — it has no
 * hue to wash, so it takes a solid fill, never an outline (R-27), and the
 * STRONG step of the two (`--background-fill-strong`), not the rest fill a field
 * or a secondary button takes. A badge sits on rows that wash on hover, and the
 * hover wash is the rest fill's value: on the rest fill a "Built-in" badge
 * vanished into the row it labels the moment the pointer arrived (measured in
 * the primitives gallery, 2026-10-10). The strong step stays a step off the
 * page, the hovered row and a filled card, and inside a dark popover it is
 * re-pointed one step deeper than the ground, so the badge reads there too.
 */
const toneClass = {
  neutral: 'bg-background-fill-strong text-text-muted',
  accent: 'bg-background-accent/22 text-text-accent',
  info: 'bg-background-info/22 text-text-info',
  success: 'bg-background-success/22 text-text-success',
  warning: 'bg-background-warning/22 text-text-warning',
  danger: 'bg-background-danger/22 text-text-danger',
} as const;

const variantClass = {
  badge: 'h-5 px-1.5',
  chip: 'h-6 px-2',
} as const;

export type BadgeTone = keyof typeof toneClass;
export type BadgeVariant = keyof typeof variantClass;

export interface BadgeProps extends React.ComponentProps<'span'> {
  tone?: BadgeTone;
  /** `badge` (20px, status) or `chip` (24px, category/filter). */
  variant?: BadgeVariant;
  /** Uppercase micro-label styling, for tags like “KB”, “Focused”. */
  uppercase?: boolean;
  /**
   * Render the chip's own child element instead of a `<span>` — so a TOGGLE
   * chip IS the button rather than a `<span>` wrapped in one.
   *
   * ⚠ This is not a convenience. The global focus rule (design system D-15)
   * paints `background-color: var(--background-focus)` on the focused
   * `<button>`, and `tone="neutral"` is `bg-background-fill-strong` — an OPAQUE fill.
   * A `<button><Badge/></button>` therefore covers its own focus surface
   * completely and a keyboard user gets no focus indication at all. With
   * `asChild` the fill and the focus surface are the same box, so the focus
   * treatment reads. It also keeps toggle chips out of the raw-`<button>`
   * backlog (DR-24). P4: if a surface needs a variant, the variant lives in the
   * primitive.
   *
   * The pressed state is `tint-selected tint-interactive`, never
   * `tone="accent"`: the tints are translucent and the focus fill reads through
   * them, while an opaque accent tone reintroduces the problem this prop exists
   * to solve.
   */
  asChild?: boolean;
}

export function Badge({
  tone = 'neutral',
  variant = 'badge',
  uppercase = false,
  asChild = false,
  className,
  ...props
}: BadgeProps) {
  const Comp = asChild ? Slot : 'span';

  return (
    <Comp
      className={cn(
        'inline-flex flex-shrink-0 items-center gap-1 rounded-inner text-chip',
        variantClass[variant],
        // `text-caps` IS the caps style — it carries the uppercase transform, the
        // +0.04em tracking and the 500 weight together, so there is nothing to
        // add beside it. It shares the `text` merge group with `text-chip` above,
        // which is why the override lands instead of both surviving.
        uppercase && 'text-caps',
        toneClass[tone],
        className
      )}
      {...props}
    />
  );
}
