import * as React from 'react';

import { cn } from '../../utils';

/**
 * The attributes of a field that holds a MACHINE string: a command (`claude`), a URL, a model id
 * (`gpt-5.5-2026-04-24`), a region (`us-west-2`), an environment variable or header name, a
 * config value. Vision V4-R7-5.
 *
 * The browser's spellchecker knows nothing about those, so on a field left as it ships Chromium
 * draws a red dotted misspelling line under a correct value, which tells a person something is
 * wrong with it. The same trio keeps the platform from "correcting" the value (`autoCorrect`) or
 * turning the first letter of a command into a capital (`autoCapitalize`).
 *
 * Use it through {@link InputProps.machine} (`<Input machine />`). It is exported for the one
 * place that cannot: a props object built before the element exists (the provider form's field
 * table spreads it). A secret field needs nothing, `SecretInput` already switches spellcheck off.
 * A field that holds a person's own words (a name, a description, a chat message) keeps its
 * spellcheck, so it never takes this.
 */
export const machineStringProps = {
  spellCheck: false,
  autoCorrect: 'off',
  autoCapitalize: 'off',
} as const;

type InputProps = React.ComponentProps<'input'> & {
  /**
   * The field holds a machine string, not words: spellcheck, auto-correct and auto-capitalise are
   * off ({@link machineStringProps}). Placed before the caller's own props, so an explicit
   * `spellCheck` still wins.
   */
  machine?: boolean;
};

/**
 * The text field, on the control ladder (§3.2).
 *
 * Height is the 32px md rung — the same box a Button, a Select trigger and a
 * menu row take, so a form row and a toolbar cluster line up without anyone
 * measuring. Text inset is 8px, which is what the wrapper construction produces
 * for a field with no leading icon. The value is `text-body` (14/20/400, the
 * role for values, as in `Textarea`) in `--text-default`: the field sets its own
 * ink, so a value never inherits a muted container's and reads as a placeholder
 * (first run, FR-R1-10: the prefilled Endpoint inside the muted tab panel).
 *
 * FILLS BEFORE LINES (owner message 13, R-27, design.md 9.11). The field is a
 * subtle solid fill with no resting outline: the control ground at rest
 * (`--background-fill`), `--background-fill-strong` under the pointer, and the
 * 1px border kept but transparent, so nothing moves when an edge appears. The
 * whole look is authored
 * in `main.css` (`.br-field`, layered so a caller's utility still wins), because
 * the edges it draws are load-bearing: `--border-focus` while focused (D-15: a
 * field firms an edge, never a ring), `--border-danger` while invalid, and the
 * system ink in forced colours, which draw the transparent border. Disabled is
 * the same fill at half opacity.
 *
 * Inside a filled box (a Note, a Card, a dialog's inner panel, a caller's
 * `.br-fill-surface`) the fill steps back to the page ground by itself, so a field
 * never sits fill on fill.
 */
const Input = React.forwardRef<HTMLInputElement, InputProps>(
  ({ className, type, machine, ...props }, ref) => {
    return (
      <input
        type={type}
        {...(machine ? machineStringProps : null)}
        className={cn(
          'br-field flex h-control-md w-full rounded-element border px-2 text-body text-text-default transition-[color,background-color,border-color]',
          'file:border-0 file:bg-transparent file:text-label file:text-text-default placeholder:text-text-muted',
          'aria-invalid:border-border-danger disabled:cursor-not-allowed disabled:opacity-50',
          className
        )}
        ref={ref}
        {...props}
      />
    );
  }
);
Input.displayName = 'Input';

export { Input };
