import * as React from 'react';

import { cn } from '../../utils';
import { Eye, EyeOff } from '../icons/app-icons';
import { Button } from './button';
import { Input } from './input';

type SecretInputProps = Omit<React.ComponentProps<'input'>, 'type'> & {
  /**
   * What the field holds, in words — it names the reveal toggle for a screen
   * reader ("Show Secret Access Key"), which otherwise hears only an eye.
   */
  revealLabel: string;
};

/**
 * The field a credential is typed into: an API key, a secret access key, a
 * token. Masked until the person at the keyboard asks to see it.
 *
 * ⚠ **The variant lives here, not at the call sites** (design.md P4). The two
 * provider forms used to disagree — the custom-provider form masked its key and
 * the form every built-in provider shares rendered every parameter, secrets
 * included, as plain `type="text"` with spellcheck on, so a Secret Access Key was
 * on screen and in the DOM as it was typed. That was a divergence, not a
 * decision, and one primitive is what stops it recurring in a third form.
 *
 * What the primitive guarantees, whatever a caller passes:
 * - `type="password"` until the toggle is pressed, and again on every mount — a
 *   reopened dialog starts masked.
 * - `autoComplete="off"` and `spellCheck={false}`, applied AFTER the caller's
 *   props so neither can be switched back on. Spellcheck matters in the revealed
 *   state: a checked `text` field hands the value to the platform dictionary.
 *
 * The toggle only ever reveals what was typed into this field in this session.
 * A stored secret is never loaded back into it — the forms show the daemon's
 * masked form as a placeholder instead — so there is nothing here that could
 * read one back out of the credential store.
 */
const SecretInput = React.forwardRef<HTMLInputElement, SecretInputProps>(
  ({ className, revealLabel, disabled, ...props }, ref) => {
    const [revealed, setRevealed] = React.useState(false);

    return (
      <div className="relative w-full">
        <Input
          ref={ref}
          {...props}
          disabled={disabled}
          // Room for the toggle, so a long key never runs underneath it.
          className={cn(className, 'pr-9')}
          type={revealed ? 'text' : 'password'}
          autoComplete="off"
          spellCheck={false}
        />
        <Button
          type="button"
          variant="ghost"
          size="xs"
          shape="round"
          disabled={disabled}
          onClick={() => setRevealed((current) => !current)}
          aria-label={`${revealed ? 'Hide' : 'Show'} ${revealLabel}`}
          aria-pressed={revealed}
          className="absolute right-1.5 top-1/2 -translate-y-1/2 text-text-muted"
        >
          {revealed ? <EyeOff className="size-3.5" /> : <Eye className="size-3.5" />}
        </Button>
      </div>
    );
  }
);
SecretInput.displayName = 'SecretInput';

export { SecretInput };
