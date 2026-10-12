import * as React from 'react';
import * as SwitchPrimitives from '@radix-ui/react-switch';
import { cn } from '../../utils';

/**
 * The switch (spec 2.6): ONE style, no variants. Use it when the change applies now, on the
 * right of its row (principle 5).
 *
 * - Track 32×20, fully round. Off: `--control-track-off`. On: `--background-accent` (the
 *   on-state is one of the accent's named places, principle 7).
 * - Knob 16px, inset 2px on every side in BOTH states, travel 12px, and it does not grow (the
 *   old 16 → 20px growth left the off knob 2px from the side and 4px from the top and bottom).
 *   Off: `--background-default` ringed by `--border-control` (3:1 against the off track; the old
 *   white knob on `--background-strong` measured 1.38:1), the same in dark: a `--text-muted` dark
 *   knob read as the brighter, "on" state (coordinator ruling 8). On: `--text-on-accent`.
 * - A 24px hit target: the 20px track carries a 2px invisible extension above and below. In a
 *   `<label>` row (or a `SettingRow`), the whole row toggles it.
 * - Motion: knob `transform` and track `background-color` over 125ms `--ease-out`. A value that
 *   changes without the person touching the switch (it loaded asynchronously, another window
 *   changed it) snaps: the render that shows it carries `data-motion-still`, so a settings page
 *   that fills in after mount does not play a row of slides.
 *
 *   ⚠ **The flag is derived DURING the render that flips `data-state`, never in an effect after
 *   it (vision round 1, V4-01).** It used to be set by a layout effect, one commit later. Any
 *   style read between the two commits (a sibling's layout effect measuring: a SegmentedControl
 *   in the same section re-measures on every render) made the browser compute the new
 *   `data-state` with the 125ms duration and START the transition, and a running transition
 *   ignores a later `transition-duration: 0ms`. Measured on Settings > App: "Prevent sleep while
 *   running" slid 14 → 5.7 → 4 → 2 every time the tab opened. The flag is released two frames
 *   later, so at least one frame is styled with it. `controlsGeometry.browser.test.tsx` pins it.
 * - Name: the visible row label, through `aria-labelledby` (or a `<label htmlFor>`). Never
 *   "Toggle …" or "Enable …", and never a name that changes with the state.
 * - Geometry and colour are authored CSS (`.br-switch` in `main.css`), so they cannot fail to
 *   generate the way a newly written utility can.
 * - Focus keeps the track's colour (it is the state) and draws the inset 2px `--border-focus`
 *   edge of D-15.
 */
/** How long after a toggle a change of `checked` still counts as the person's (a slow save). */
export const SWITCH_TOUCH_WINDOW_MS = 4000;

/** There is one switch, so there is no `variant`: the prop was deleted in wave 2 (spec 2.6). */
export type SwitchProps = React.ComponentPropsWithoutRef<typeof SwitchPrimitives.Root>;

export const Switch = React.forwardRef<React.ElementRef<typeof SwitchPrimitives.Root>, SwitchProps>(
  ({ className, checked, onCheckedChange, ...props }, ref) => {
    // When the person last toggled it. A change that follows within the window is theirs, even
    // when the caller saves first and updates `checked` once the save answers.
    const touchedAt = React.useRef<number | null>(null);
    // The value the last render showed, and a token (> 0) while a change that was NOT the
    // person's is held at rest. A fresh token per change restarts the release.
    const [shown, setShown] = React.useState(checked);
    const [still, setStill] = React.useState(0);

    if (checked !== shown) {
      // Derived state, set during render: React re-renders before committing, so the new
      // `data-state` and `data-motion-still` reach the DOM in the same commit. Reading the ref is
      // pure; it is cleared after the commit below.
      setShown(checked);
      const touched =
        touchedAt.current !== null && Date.now() - touchedAt.current < SWITCH_TOUCH_WINDOW_MS;
      setStill(touched ? 0 : still + 1);
    }

    React.useLayoutEffect(() => {
      // The change this render showed has been judged; the next one is judged afresh.
      touchedAt.current = null;
    }, [checked]);

    React.useEffect(() => {
      if (!still) return;
      // Two frames: the first is styled with the flag (the snap), the second releases it.
      let second = 0;
      const first = window.requestAnimationFrame(() => {
        second = window.requestAnimationFrame(() => setStill(0));
      });
      return () => {
        window.cancelAnimationFrame(first);
        window.cancelAnimationFrame(second);
      };
    }, [still]);

    const callerStill = (props as Record<string, unknown>)['data-motion-still'];

    return (
      <SwitchPrimitives.Root
        data-slot="switch"
        className={cn('br-switch peer', className)}
        checked={checked}
        onCheckedChange={(next) => {
          touchedAt.current = Date.now();
          onCheckedChange?.(next);
        }}
        {...props}
        data-motion-still={still > 0 || callerStill !== undefined ? '' : undefined}
        ref={ref}
      >
        <SwitchPrimitives.Thumb data-slot="switch-thumb" className="br-switch-thumb" />
      </SwitchPrimitives.Root>
    );
  }
);
Switch.displayName = SwitchPrimitives.Root.displayName;
