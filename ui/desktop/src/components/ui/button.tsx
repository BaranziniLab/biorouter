import * as React from 'react';
import { Slot } from '@radix-ui/react-slot';
import { cva, type VariantProps } from 'class-variance-authority';
import { cn } from '../../utils';

const buttonVariants = cva(
  // Focus is handled globally in main.css (:focus-visible ring); no per-component ring here.
  // transform is in the transition list so the shared active:scale press eases on
  // release (and any hover:scale on a call site animates instead of snapping).
  // NB: Tailwind v4 maps scale-*/hover:scale-*/active:scale-* to the standalone
  // `scale` property (not `transform`), so `scale` must be in the transition list
  // for the press/hover scale to ease rather than snap.
  // `--tint-ink` is in the list for the same class of reason: four of the five
  // variants take their hover/press from `tint-interactive`, which carries the
  // tint in that registered custom property (background-image does not animate in
  // Chrome — see main.css). Omitting it made `default` ease its own fill while
  // destructive/outline/secondary/ghost snapped: two hover behaviours in one
  // component. An explicit `transition-[…]` list REPLACES the utility's own
  // fallback, so this is the only place the button can declare it.
  // `text-label` IS 14/20/500 — it replaces `text-sm font-medium` exactly, and is
  // the one type role every control in the app now shares.
  // The duration/easing annotations are gone: `--default-transition-duration` and
  // `--default-transition-timing-function` are already `--dur-fast` / `--ease-out`
  // in main.css, so any `transition-*` utility picks them up unannotated.
  "inline-flex items-center justify-center gap-2 whitespace-nowrap text-label transition-[color,background-color,border-color,transform,scale,opacity,--tint-ink] active:scale-[0.98] cursor-pointer disabled:pointer-events-none disabled:opacity-50 [&_svg]:pointer-events-none [&_svg:not([class*='size-'])]:size-4 shrink-0 [&_svg]:shrink-0",
  {
    variants: {
      // One state stack, four fills (§3.1). Hover and press are NOT authored per
      // variant any more: `tint-interactive` carries both, as a background-IMAGE
      // that composites over whatever fill the variant already set. This is the
      // whole reason a filled control may not use `bg-overlay-*` — a background
      // COLOUR replaces the fill, so `secondary` would get LIGHTER on hover
      // (5% ink over the PARENT's ground) instead of darker.
      //
      // The two exceptions are deliberate and both are solid accent/hue fills:
      // `default` brightens its own token (`--background-accent-hover`), which is
      // §3.1's "solid fills brighten rather than take an overlay"; `link` has no
      // box to tint at all.
      variant: {
        default:
          'bg-background-accent text-text-on-accent hover:bg-background-accent-hover biorouter-focus-surface-accent',
        // Not a solid red block (§3.1). A translucent danger wash with the deep
        // danger ink on top reads as consequence without shouting, and — because
        // the fill is now translucent rather than opaque — the tint composites
        // correctly. This also retires `hover:opacity-90`, which faded the whole
        // button and let a modal scrim ghost straight through it.
        destructive:
          'bg-background-danger/22 text-text-danger tint-interactive biorouter-focus-surface',
        outline:
          'bg-transparent border border-border-emphasized text-text-default tint-interactive biorouter-focus-surface',
        secondary:
          'bg-background-medium text-text-default tint-interactive biorouter-focus-surface',
        ghost: 'bg-transparent text-text-default tint-interactive biorouter-focus-surface',
        link: 'text-text-accent underline-offset-4 hover:underline',
      },
      // The control ladder (§2.3): sm 28 / md 32 / lg 36, with `default` = md,
      // plus the sanctioned 24px compact tier as `xs`. Font size does NOT change
      // with height — every rung is `text-label` — so only the box moves. This
      // retires the old 24/32/36/40 ladder and, with it, the off-scale 28px that
      // row actions were hand-rolling because no rung offered it.
      size: {
        xs: 'h-control-compact gap-1 [&_svg:not([class*=size-])]:size-3',
        default: 'h-control-md',
        sm: 'h-control-sm gap-1.5',
        lg: 'h-control-lg',
      },
      // 'pill' is a misnomer — it maps to rounded-element (8px), not a full pill.
      // 'round' is a square icon button (w==h via compound variants), also rounded-element.
      shape: {
        pill: 'rounded-element',
        round: '',
      },
    },
    // Horizontal padding is 12px at every rung (§3.1) — the four values it
    // replaces (16/16/24 plus three `has-[>svg]` forks) were the reason two
    // buttons of the same height could still be different widths for the same
    // label. Vertical padding is gone entirely: the height token owns the box and
    // `items-center` owns the 20px line inside it, which is what lets a rung stay
    // on the ladder when a call site adds an icon.
    compoundVariants: [
      {
        shape: 'pill',
        size: 'xs',
        className: 'px-2',
      },
      {
        shape: 'pill',
        size: 'default',
        className: 'px-3',
      },
      {
        shape: 'pill',
        size: 'sm',
        className: 'px-3',
      },
      {
        shape: 'pill',
        size: 'lg',
        className: 'px-3',
      },
      // Icon buttons are square on the same ladder: 32×32 with a 16px icon is the
      // one row-action size, and 24×24 is the compact tier (code-block chrome).
      {
        shape: 'round',
        size: 'xs',
        className: 'w-control-compact h-control-compact p-0 rounded-element',
      },
      {
        shape: 'round',
        size: 'default',
        className: 'w-control-md h-control-md p-0 rounded-element',
      },
      {
        shape: 'round',
        size: 'sm',
        className: 'w-control-sm h-control-sm p-0 rounded-element',
      },
      {
        shape: 'round',
        size: 'lg',
        className: 'w-control-lg h-control-lg p-0 rounded-element',
      },
    ],
    defaultVariants: {
      variant: 'default',
      size: 'default',
      shape: 'pill',
    },
  }
);

const Button = React.forwardRef<
  HTMLButtonElement,
  React.ComponentProps<'button'> &
    VariantProps<typeof buttonVariants> & {
      asChild?: boolean;
      shape?: 'pill' | 'round';
    }
>(({ className, variant, size, asChild = false, shape = 'pill', ...props }, ref) => {
  const Comp = asChild ? Slot : 'button';

  return (
    <Comp
      data-slot="button"
      className={cn(buttonVariants({ variant, size, shape, className }))}
      ref={ref}
      {...props}
    />
  );
});

Button.displayName = 'Button';

export { Button, buttonVariants };
