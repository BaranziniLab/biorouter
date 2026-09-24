import * as React from 'react';

import { cn } from '../../utils';
import { Check, Copy, Eye, EyeOff } from '../icons/app-icons';
import { Button } from './button';

export interface CopyFieldProps {
  /** Exactly what is copied — never the display form. */
  value: string;
  /** What the value is, in words. The Copy button is named "Copy {label}". */
  label: string;
  /** How the value is shown, when that differs from what is copied (grouped codes). */
  display?: string;
  /** Wraps and keeps newlines (an invitation message); Copy sits top-right. */
  multiline?: boolean;
  /** Single-line overflow. The full value is still in the DOM and still copied. */
  truncate?: 'end' | 'middle';
  /** Masked with a reveal toggle. Copy works while masked. */
  secret?: boolean;
  /** `code` sets the value at 20/28 mono — for a device code someone reads aloud. */
  size?: 'default' | 'code';
  onCopied?: () => void;
  /** Layout only (width, margins). The box is the primitive's. */
  className?: string;
}

/** How long "Copied" / "Copy failed" stays on the button. */
export const COPY_FIELD_FEEDBACK_MS = 2000;

/** The characters a middle truncation keeps at the end — a file name's worth. */
const MIDDLE_TAIL_LENGTH = 12;

/** A fixed-width mask, so the mask does not disclose the secret's length. */
const SECRET_MASK = '••••••••••••';

type Feedback = 'idle' | 'copied' | 'failed';

async function writeClipboard(text: string): Promise<void> {
  // `navigator.clipboard` can be absent (an insecure context) as well as
  // rejecting (no permission, a document without focus), so it is a check AND a
  // catch at the call site.
  if (!navigator.clipboard?.writeText) throw new Error('clipboard unavailable');
  await navigator.clipboard.writeText(text);
}

/**
 * The one box for everything a person hands to someone else: an invitation
 * message, a device code, a fingerprint, a command, a server path.
 *
 * - One click copies `value` (never `display`). The button reads "Copied" with a
 *   settling check for two seconds; a polite live region says "Copied". No
 *   toast — the confirmation is where the person is looking.
 * - If the clipboard refuses, the button reads "Copy failed" and the value is
 *   selected, so ⌘C works. A masked secret is revealed first: selecting the mask
 *   would put bullets on the clipboard, and the person asked to take the value.
 * - A ⌘C of the whole shown text puts `value` on the clipboard too, so a grouped
 *   display ("7QK2-M9XA-…", a fingerprint in fours) never leaks its separators
 *   into what is pasted. A partial selection copies exactly what was selected.
 * - The accessible name stays "Copy {label}" through every state, so the control
 *   never changes identity under a screen reader; the live region carries the
 *   outcome.
 *
 * The box is authored CSS (`.biorouter-copy-field` in `main.css`) on
 * `--background-well` in BOTH forms. Never `--background-code`: in dark mode it
 * equals the page, so the box vanishes inside a dialog.
 */
export function CopyField({
  value,
  label,
  display,
  multiline = false,
  truncate,
  secret = false,
  size = 'default',
  onCopied,
  className,
}: CopyFieldProps) {
  const [feedback, setFeedback] = React.useState<Feedback>('idle');
  const [announcement, setAnnouncement] = React.useState('');
  // Every mount starts masked: a reopened dialog never shows a secret it was
  // not asked to show.
  const [revealed, setRevealed] = React.useState(false);
  const [selectPending, setSelectPending] = React.useState(false);
  const valueRef = React.useRef<HTMLSpanElement>(null);
  const timerRef = React.useRef<ReturnType<typeof setTimeout> | null>(null);

  React.useEffect(
    () => () => {
      if (timerRef.current) clearTimeout(timerRef.current);
    },
    []
  );

  const selectValue = React.useCallback(() => {
    const node = valueRef.current;
    const selection = typeof window !== 'undefined' ? window.getSelection() : null;
    if (!node || !selection) return;
    selection.selectAllChildren(node);
  }, []);

  // A reveal re-renders the value first; select it only once it is on screen.
  React.useEffect(() => {
    if (!selectPending) return;
    setSelectPending(false);
    selectValue();
  }, [selectPending, selectValue]);

  const settle = (next: Feedback) => {
    if (timerRef.current) clearTimeout(timerRef.current);
    setFeedback(next);
    setAnnouncement(next === 'copied' ? 'Copied' : 'Copy failed');
    timerRef.current = setTimeout(() => {
      timerRef.current = null;
      setFeedback('idle');
      setAnnouncement('');
    }, COPY_FIELD_FEEDBACK_MS);
  };

  const copy = async () => {
    try {
      await writeClipboard(value);
    } catch {
      settle('failed');
      if (secret && !revealed) {
        setRevealed(true);
        setSelectPending(true);
      } else {
        selectValue();
      }
      return;
    }
    settle('copied');
    onCopied?.();
  };

  const masked = secret && !revealed;
  const shown = display ?? value;

  // `user-select: all` makes one click take the whole value, and the clipboard
  // fallback selects it on purpose; either way a ⌘C of the WHOLE shown form
  // should yield the value. The listener sits on the root because a copy event
  // targets the focused element when there is one — after a failed click that
  // is the Copy button, not the value.
  const handleCopyEvent = (event: React.ClipboardEvent<HTMLDivElement>) => {
    if (masked || shown === value) return;
    const selection = typeof window !== 'undefined' ? window.getSelection() : null;
    const node = valueRef.current;
    if (!selection || !node || !node.contains(selection.anchorNode)) return;
    if (selection.toString() !== shown) return;
    event.preventDefault();
    event.clipboardData.setData('text/plain', value);
  };
  const singleLineTruncate = multiline ? undefined : truncate;

  let valueContent: React.ReactNode;
  if (masked) {
    valueContent = (
      <>
        <span aria-hidden="true">{SECRET_MASK}</span>
        <span className="sr-only">Hidden</span>
      </>
    );
  } else if (singleLineTruncate === 'middle' && Array.from(shown).length > MIDDLE_TAIL_LENGTH) {
    const characters = Array.from(shown);
    const head = characters.slice(0, -MIDDLE_TAIL_LENGTH).join('');
    const tail = characters.slice(-MIDDLE_TAIL_LENGTH).join('');
    valueContent = (
      <>
        <span className="biorouter-copy-field-head">{head}</span>
        <span className="biorouter-copy-field-tail">{tail}</span>
      </>
    );
  } else {
    valueContent = shown;
  }

  return (
    <div
      data-slot="copy-field"
      data-multiline={multiline ? 'true' : undefined}
      data-size={size}
      className={cn('biorouter-copy-field', className)}
      onCopy={handleCopyEvent}
    >
      <span
        ref={valueRef}
        className="biorouter-copy-field-value"
        data-truncate={singleLineTruncate}
        data-masked={masked ? 'true' : undefined}
      >
        {valueContent}
      </span>
      <span className="biorouter-copy-field-actions">
        {secret ? (
          <Button
            type="button"
            variant="ghost"
            size="sm"
            shape="round"
            onClick={() => setRevealed((current) => !current)}
            aria-label={`${revealed ? 'Hide' : 'Show'} ${label}`}
            aria-pressed={revealed}
            className="text-text-muted"
          >
            {revealed ? <EyeOff /> : <Eye />}
          </Button>
        ) : null}
        <Button
          type="button"
          variant="ghost"
          size="sm"
          onClick={() => void copy()}
          aria-label={`Copy ${label}`}
          data-feedback={feedback}
        >
          {feedback === 'copied' ? (
            <>
              <Check aria-hidden className="biorouter-check-settled" />
              Copied
            </>
          ) : (
            <>
              <Copy aria-hidden />
              {feedback === 'failed' ? 'Copy failed' : 'Copy'}
            </>
          )}
        </Button>
      </span>
      <span className="sr-only" aria-live="polite" aria-atomic="true">
        {announcement}
      </span>
    </div>
  );
}
