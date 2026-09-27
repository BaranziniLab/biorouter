import * as React from 'react';
import { Input } from '../../ui/input';
import { cn } from '../../../utils';
import { deviceCodeCopy } from './copy';
import {
  caretAfterCodeCharacters,
  codeCharactersBefore,
  deviceCodeProblem,
  groupDeviceCodeInput,
  normalizeDeviceCodeInput,
} from './deviceCode';

export interface DeviceCodeInputProps extends Omit<
  React.ComponentProps<'input'>,
  'value' | 'onChange' | 'type' | 'defaultValue'
> {
  /** The code as the broker compares it: normalized, ungrouped (`7QK2M9XA3JTPWZ4D`). */
  value: string;
  onChange(code: string): void;
}

/**
 * The one field a host pastes a joiner's device code into (ui-redesign-spec, "Invite and admit").
 *
 * - It takes the code however it arrives — `7QK2-M9XA-3JTP-WZ4D`, `7qk2m9xa3jtpwz4d`, with spaces —
 *   and normalizes it as the broker will (`deviceCode.ts`), showing it grouped in fours.
 * - A code that cannot be one (a `U`, a stray character, the wrong length) sets the field's native
 *   validity, so the form's own `required` validation blocks the submit with the reason.
 * - It never shows a code the host did not type or paste: it has no default value and no source
 *   of codes of its own.
 *
 * `autoComplete="one-time-code"` lets a platform offer a code it received, which is the same act
 * as pasting it.
 */
export const DeviceCodeInput = React.forwardRef<HTMLInputElement, DeviceCodeInputProps>(
  ({ value, onChange, className, onBlur, ...props }, forwardedRef) => {
    const inner = React.useRef<HTMLInputElement | null>(null);
    const caret = React.useRef<number | null>(null);
    const grouped = groupDeviceCodeInput(value);
    const problem = value ? deviceCodeProblem(value) : null;

    const setRef = (node: HTMLInputElement | null) => {
      inner.current = node;
      if (typeof forwardedRef === 'function') forwardedRef(node);
      else if (forwardedRef) forwardedRef.current = node;
    };

    // Native validity carries the reason, so `required` + the form's own validation refuse a code
    // that cannot be one, and the browser says why.
    React.useEffect(() => {
      inner.current?.setCustomValidity(problem ?? '');
    }, [problem]);

    // Regrouping moves text around the caret; put it back after the same code character.
    React.useLayoutEffect(() => {
      const node = inner.current;
      if (node === null || caret.current === null) return;
      const offset = caret.current;
      caret.current = null;
      if (typeof document !== 'undefined' && document.activeElement === node)
        node.setSelectionRange(offset, offset);
    }, [grouped]);

    const handleChange = (event: React.ChangeEvent<HTMLInputElement>) => {
      const raw = event.target.value;
      const at = event.target.selectionStart ?? raw.length;
      let before = codeCharactersBefore(raw, at);
      let next = normalizeDeviceCodeInput(raw);
      // Deleting only a group's hyphen changes no code character, and regrouping would put the
      // hyphen straight back: delete the character before it instead, as the person meant.
      if (next === value && raw.length < grouped.length && before > 0) {
        const chars = Array.from(next);
        chars.splice(before - 1, 1);
        next = chars.join('');
        before -= 1;
      }
      caret.current = caretAfterCodeCharacters(groupDeviceCodeInput(next), before);
      onChange(next);
    };

    return (
      <Input
        ref={setRef}
        type="text"
        value={grouped}
        onChange={handleChange}
        onBlur={onBlur}
        autoComplete="one-time-code"
        autoCapitalize="characters"
        autoCorrect="off"
        spellCheck={false}
        translate="no"
        inputMode="text"
        placeholder={deviceCodeCopy.placeholder}
        className={cn('font-mono tracking-wide', className)}
        {...props}
      />
    );
  }
);
DeviceCodeInput.displayName = 'DeviceCodeInput';
