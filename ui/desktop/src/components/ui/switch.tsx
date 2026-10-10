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
 *   white knob on `--background-strong` measured 1.38:1), with `--text-muted` as the dark fill.
 *   On: `--text-on-accent`.
 * - A 24px hit target: the 20px track carries a 2px invisible extension above and below. In a
 *   `<label>` row (or a `SettingRow`), the whole row toggles it.
 * - Motion: knob `transform` and track `background-color` over 125ms `--ease-out`. A value that
 *   changes without the person touching the switch (it loaded asynchronously, another window
 *   changed it) snaps: that commit carries `data-motion-still`, so a settings page that fills in
 *   after mount does not play a row of slides.
 * - Name: the visible row label, through `aria-labelledby` (or a `<label htmlFor>`). Never
 *   "Toggle …" or "Enable …", and never a name that changes with the state.
 * - Geometry and colour are authored CSS (`.br-switch` in `main.css`), so they cannot fail to
 *   generate the way a newly written utility can.
 * - Focus keeps the track's colour (it is the state) and draws the inset 2px `--border-focus`
 *   edge of D-15.
 */
/** How long after a toggle a change of `checked` still counts as the person's (a slow save). */
export const SWITCH_TOUCH_WINDOW_MS = 4000;

export type SwitchProps = React.ComponentPropsWithoutRef<typeof SwitchPrimitives.Root> & {
  /**
   * @deprecated Ignored: there is one switch. Accepted until every owner has removed it from
   * their call sites (wave 2), then deleted.
   */
  variant?: 'default' | 'mono';
};

export const Switch = React.forwardRef<React.ElementRef<typeof SwitchPrimitives.Root>, SwitchProps>(
  ({ className, variant: _variant, checked, onCheckedChange, ...props }, ref) => {
    // When the person last toggled it. A change that follows within the window is theirs, even
    // when the caller saves first and updates `checked` once the save answers.
    const touchedAt = React.useRef<number | null>(null);
    const previous = React.useRef(checked);
    const [still, setStill] = React.useState(false);

    React.useLayoutEffect(() => {
      if (previous.current === checked) return;
      previous.current = checked;
      const touched =
        touchedAt.current !== null && Date.now() - touchedAt.current < SWITCH_TOUCH_WINDOW_MS;
      touchedAt.current = null;
      if (!touched) setStill(true);
    }, [checked]);

    React.useEffect(() => {
      if (!still) return;
      const frame = window.requestAnimationFrame(() => setStill(false));
      return () => window.cancelAnimationFrame(frame);
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
        data-motion-still={still || callerStill !== undefined ? '' : undefined}
        ref={ref}
      >
        <SwitchPrimitives.Thumb data-slot="switch-thumb" className="br-switch-thumb" />
      </SwitchPrimitives.Root>
    );
  }
);
Switch.displayName = SwitchPrimitives.Root.displayName;
