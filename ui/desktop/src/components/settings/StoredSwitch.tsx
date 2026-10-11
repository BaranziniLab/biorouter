import * as React from 'react';
import { Switch, type SwitchProps } from '../ui/switch';

export type StoredSwitchProps = Omit<SwitchProps, 'checked'> & {
  /** The stored value, or `null` while it has not been read yet. */
  checked: boolean | null;
};

/**
 * A switch for a setting that lives somewhere else (the main process, the daemon) and is read
 * after the page mounts.
 *
 * Until the value is known the switch is the same 32x20 control, held at its place and not
 * painted: hidden, disabled and out of the tab order. It does not start from a guess. A
 * placeholder `true` painted the coral ON track for the two frames before the stored `false`
 * arrived, on every open of Settings > App (vision round 1 V4-01, round 2 re-report), and a
 * placeholder `false` would flash the opposite lie for the person who has it on. Holding the
 * place, rather than swapping a skeleton in, is what keeps the row exactly the height and width
 * it has once the value lands, so nothing below it moves.
 *
 * The value then arrives as a change the person did not make, so `Switch` snaps the knob there
 * (`data-motion-still`) instead of sliding it.
 */
export const StoredSwitch = React.forwardRef<React.ElementRef<typeof Switch>, StoredSwitchProps>(
  ({ checked, disabled, style, ...props }, ref) => {
    const pending = checked === null;
    return (
      <Switch
        {...props}
        ref={ref}
        checked={checked === true}
        disabled={pending || disabled}
        style={pending ? { ...style, visibility: 'hidden' } : style}
        data-pending={pending ? '' : undefined}
      />
    );
  }
);
StoredSwitch.displayName = 'StoredSwitch';
