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
  /**
   * A class for the value's own box, so an area's stylesheet can change how the value flows (Crew
   * shows install commands unwrapped, scrolling sideways, so no line breaks mid-word).
   */
  valueClassName?: string;
}

/** How long "Copied" / "Copy failed" stays on the button. */
export const COPY_FIELD_FEEDBACK_MS = 2000;

/** The characters a middle truncation keeps at the end — a file name's worth. */
const MIDDLE_TAIL_LENGTH = 12;

/** A fixed-width mask, so the mask does not disclose the secret's length. */
const SECRET_MASK = '••••••••••••';

/**
 * The labels' shared cell. Inline styles, not utilities: a newly written utility can silently fail
 * to generate (see `CLAUDE.md`, "Desktop shell geometry"), and this is load-bearing geometry.
 */
const LABEL_STACK_STYLE: React.CSSProperties = { display: 'inline-grid' };
const LABEL_CELL_STYLE: React.CSSProperties = { gridArea: '1 / 1' };
const LABEL_HIDDEN_STYLE: React.CSSProperties = { gridArea: '1 / 1', visibility: 'hidden' };

/** One of the Copy button's labels, in the shared cell; an inactive one keeps its width only. */
function CopyLabel({ active, children }: { active: boolean; children: React.ReactNode }) {
  return (
    <span
      data-active={active ? 'true' : undefined}
      aria-hidden={active ? undefined : true}
      className="inline-flex items-center justify-center gap-1.5"
      style={active ? LABEL_CELL_STYLE : LABEL_HIDDEN_STYLE}
    >
      {children}
    </span>
  );
}

type Feedback = 'idle' | 'copied' | 'failed';

/**
 * A multi-line value longer than this shows its first four lines behind a fade, with "Show all"
 * (QA Q3-37). The Invite dialog's invitation — 4 lines of instructions and 16 of base64, 724
 * characters — was the largest thing in the dialog and looked like something to read. The host's
 * start commands and the install commands stay whole, because someone reviews those before running
 * them: the start commands measure 442 with the longest (40-character) workspace name, and
 * `copy-field.test.tsx` fails if either set ever crosses this line.
 */
export const COPY_FIELD_CLAMP_CHARS = 600;

/** The line under a clamped value, so nobody thinks Copy takes only what shows. */
export const COPY_FIELD_CLAMP_NOTE = 'The whole message is copied.';

/** How long a refused clipboard write waits, after focusing the window, before its one retry. */
export const COPY_FIELD_RETRY_DELAY_MS = 50;

async function writeClipboard(text: string): Promise<void> {
  // `navigator.clipboard` can be absent (an insecure context) as well as
  // rejecting (no permission, a document without focus), so it is a check AND a
  // catch at the call site.
  if (!navigator.clipboard?.writeText) throw new Error('clipboard unavailable');
  await navigator.clipboard.writeText(text);
}

const wait = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));

/**
 * The last resort: select `text` in a hidden read-only textarea and ask the document to copy it.
 *
 * The textarea goes INSIDE the field (`host`), never on `<body>`: every CopyField that matters
 * sits in a dialog, whose focus trap would pull focus straight back out of `<body>`, and a copy
 * with nothing focused takes nothing. Focus goes back to what had it (the Copy button) whatever
 * happens. `false` whenever the document cannot or will not copy.
 */
function copyWithSelection(text: string, host: HTMLElement | null): boolean {
  if (!host || typeof document.execCommand !== 'function') return false;
  const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null;
  const area = document.createElement('textarea');
  area.value = text;
  area.readOnly = true;
  area.tabIndex = -1;
  area.setAttribute('aria-hidden', 'true');
  area.setAttribute('data-slot', 'copy-field-fallback');
  area.className = 'biorouter-copy-field-fallback';
  host.appendChild(area);
  let copied = false;
  try {
    area.focus({ preventScroll: true });
    area.select();
    area.setSelectionRange(0, text.length);
    copied = document.execCommand('copy') === true;
  } catch {
    copied = false;
  } finally {
    area.remove();
    previous?.focus({ preventScroll: true });
  }
  return copied;
}

/**
 * Put `text` on the clipboard, trying harder than once (QA Q3-41).
 *
 * `navigator.clipboard.writeText` rejects when the document does not have focus, which is not the
 * person's fault and usually not lasting: the Keys and security dialog's first Copy said "Copy
 * failed" once and then worked three times in a row. So a refusal focuses the window, waits a
 * beat and tries once more, and only then falls back to the selection path. "Copy failed" is left
 * for when all three have refused.
 */
async function copyText(text: string, host: HTMLElement | null): Promise<boolean> {
  try {
    await writeClipboard(text);
    return true;
  } catch {
    // Retried below.
  }
  try {
    window.focus();
  } catch {
    // A window that cannot be focused still gets its retry.
  }
  await wait(COPY_FIELD_RETRY_DELAY_MS);
  try {
    await writeClipboard(text);
    return true;
  } catch {
    // Fall back to the selection path.
  }
  return copyWithSelection(text, host);
}

/** A multi-line value wider than its box: more to the right, or scrolled to its end (Q3-18). */
type SidewaysOverflow = 'true' | 'end';

/**
 * The one box for everything a person hands to someone else: an invitation
 * message, a device code, a fingerprint, a command, a server path.
 *
 * - One click copies `value` (never `display`). The button reads "Copied" with a
 *   settling check for two seconds; a polite live region says "Copied". No
 *   toast — the confirmation is where the person is looking.
 * - If the clipboard refuses, it is asked once more after the window takes focus,
 *   then the document's own copy is tried (QA Q3-41). Only when all three refuse
 *   does the button read "Copy failed", with the value selected so ⌘C works. A
 *   masked secret is revealed first: selecting the mask would put bullets on the
 *   clipboard, and the person asked to take the value.
 * - A ⌘C of the whole shown text puts `value` on the clipboard too, so a grouped
 *   display ("7QK2-M9XA-…", a fingerprint in fours) never leaks its separators
 *   into what is pasted. A partial selection copies exactly what was selected.
 * - The accessible name stays "Copy {label}" through every state, so the control
 *   never changes identity under a screen reader; the live region carries the
 *   outcome.
 * - A multi-line value that is wider than its box (commands shown one per line,
 *   scrolling sideways) says so: a fade at the right edge, gone once it is
 *   scrolled to the end, and a scrollbar that stays visible (QA Q3-18).
 * - A multi-line value over `COPY_FIELD_CLAMP_CHARS` shows four lines behind a
 *   fade, "The whole message is copied." and "Show all" (QA Q3-37). The whole
 *   value stays in the DOM, so Copy, a click-select and ⌘C all take all of it.
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
  valueClassName,
}: CopyFieldProps) {
  const [feedback, setFeedback] = React.useState<Feedback>('idle');
  const [announcement, setAnnouncement] = React.useState('');
  // Every mount starts masked: a reopened dialog never shows a secret it was
  // not asked to show.
  const [revealed, setRevealed] = React.useState(false);
  const [selectPending, setSelectPending] = React.useState(false);
  const [expanded, setExpanded] = React.useState(false);
  const [overflow, setOverflow] = React.useState<SidewaysOverflow | undefined>(undefined);
  const rootRef = React.useRef<HTMLDivElement>(null);
  const valueRef = React.useRef<HTMLSpanElement>(null);
  const timerRef = React.useRef<ReturnType<typeof setTimeout> | null>(null);
  // A copy now awaits a retry, so the field can be gone (the dialog closed) by the time it settles.
  const mountedRef = React.useRef(false);
  const valueId = React.useId();
  const noteId = React.useId();

  React.useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      if (timerRef.current) clearTimeout(timerRef.current);
    };
  }, []);

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
    const copied = await copyText(value, rootRef.current);
    if (!mountedRef.current) return;
    if (!copied) {
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
  const clamp: 'collapsed' | 'expanded' | undefined =
    multiline && !masked && Array.from(shown).length > COPY_FIELD_CLAMP_CHARS
      ? expanded
        ? 'expanded'
        : 'collapsed'
      : undefined;

  // SIDEWAYS OVERFLOW (QA Q3-18). A caller that shows commands one per line lets the value
  // scroll sideways rather than wrap mid-flag (`white-space: pre`), and macOS hides an idle
  // scrollbar, so the host's start commands simply looked cut off mid-path — beside a line
  // promising Biorouter runs exactly these commands. Measured, not guessed: the box's width
  // follows the dialog (a ResizeObserver), its content follows the value, and the end of the
  // scroll follows the person (the value's own scroll event).
  const measureOverflow = React.useCallback(() => {
    const node = valueRef.current;
    if (!node) return;
    const hidden = node.scrollWidth - node.clientWidth;
    if (hidden <= 1) {
      setOverflow(undefined);
      return;
    }
    setOverflow(node.scrollLeft >= hidden - 1 ? 'end' : 'true');
  }, []);

  React.useLayoutEffect(() => {
    if (!multiline) return;
    const node = valueRef.current;
    if (!node) return;
    measureOverflow();
    if (typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(() => measureOverflow());
    observer.observe(node);
    return () => observer.disconnect();
  }, [multiline, measureOverflow, shown, masked, clamp]);

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
      ref={rootRef}
      data-slot="copy-field"
      data-multiline={multiline ? 'true' : undefined}
      data-overflow={multiline ? overflow : undefined}
      data-clamp={clamp}
      data-size={size}
      className={cn('biorouter-copy-field', className)}
      onCopy={handleCopyEvent}
    >
      <span
        ref={valueRef}
        id={valueId}
        className={cn('biorouter-copy-field-value', valueClassName)}
        data-truncate={singleLineTruncate}
        data-masked={masked ? 'true' : undefined}
        onScroll={multiline ? measureOverflow : undefined}
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
          aria-describedby={clamp === 'collapsed' ? noteId : undefined}
          data-feedback={feedback}
        >
          {/* All three labels share one grid cell, the inactive ones laid out but unseen, so the
              button is always as wide as the widest of them, "Copy failed", and the value beside
              it never re-wraps when the label swaps (QA Q2-24: an 18-line invitation reflowed for
              two seconds on every copy; a failed copy still did while only "Copied" was held). */}
          <span data-slot="copy-field-labels" style={LABEL_STACK_STYLE}>
            <CopyLabel active={feedback === 'idle'}>
              <Copy aria-hidden />
              Copy
            </CopyLabel>
            <CopyLabel active={feedback === 'copied'}>
              <Check
                aria-hidden
                className={feedback === 'copied' ? 'biorouter-check-settled' : undefined}
              />
              Copied
            </CopyLabel>
            <CopyLabel active={feedback === 'failed'}>
              <Copy aria-hidden />
              Copy failed
            </CopyLabel>
          </span>
        </Button>
      </span>
      {clamp ? (
        <div className="biorouter-copy-field-footer" data-slot="copy-field-footer">
          {clamp === 'collapsed' ? (
            <span id={noteId} className="biorouter-copy-field-note">
              {COPY_FIELD_CLAMP_NOTE}
            </span>
          ) : null}
          <Button
            type="button"
            variant="link"
            size="xs"
            aria-expanded={clamp === 'expanded'}
            aria-controls={valueId}
            aria-label={`${clamp === 'expanded' ? 'Show less of the' : 'Show all of the'} ${label}`}
            onClick={() => setExpanded((current) => !current)}
          >
            {clamp === 'expanded' ? 'Show less' : 'Show all'}
          </Button>
        </div>
      ) : null}
      <span className="sr-only" aria-live="polite" aria-atomic="true">
        {announcement}
      </span>
    </div>
  );
}
