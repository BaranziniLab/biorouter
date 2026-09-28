/**
 * The one copy path for every Copy control that confirms in place: `CopyField`, a fenced code
 * block's Copy (`MarkdownContent`) and the artifact panel's Copy (`ArtifactViewer`).
 *
 * The two chat/panel controls used to call `navigator.clipboard.writeText` once and, on a
 * refusal, log it (the code block) or swallow it (the panel), so a refused write looked exactly
 * like a click that did nothing. That is how 1.90.4–1.91.2 shipped a Copy that never copied: the
 * renderer's permission handler refused `clipboard-sanitized-write` (`permissionPolicy.ts`), and
 * no control said so. This path retries, then falls back to the document's own copy, and tells
 * its caller whether anything landed, so a control can say "Copy failed" instead of nothing.
 */

/** How long a refused clipboard write waits, after focusing the window, before its one retry. */
export const CLIPBOARD_RETRY_DELAY_MS = 50;

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
 * The textarea goes INSIDE the control's own box (`host`), never on `<body>`: a CopyField sits
 * in a dialog, whose focus trap would pull focus straight back out of `<body>`, and a copy with
 * nothing focused takes nothing. `.biorouter-copy-field-fallback` in `main.css` fixes it at 1px
 * and transparent, so it never shifts the layout it is dropped into. Focus goes back to what had
 * it (the Copy button) whatever happens. `false` whenever the document cannot or will not copy.
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
 * Put `text` on the clipboard, trying harder than once (QA Q3-41). `true` when it landed.
 *
 * `navigator.clipboard.writeText` rejects when the document does not have focus, which is not the
 * person's fault and usually not lasting: the Keys and security dialog's first Copy said "Copy
 * failed" once and then worked three times in a row. So a refusal focuses the window, waits a
 * beat and tries once more, and only then falls back to the selection path. A caller says "Copy
 * failed" only on `false`, when all three have refused.
 *
 * `host` is where the fallback's hidden textarea goes: the control's own box (see
 * `copyWithSelection`). `null` skips the fallback.
 */
export async function copyToClipboard(text: string, host: HTMLElement | null): Promise<boolean> {
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
  await wait(CLIPBOARD_RETRY_DELAY_MS);
  try {
    await writeClipboard(text);
    return true;
  } catch {
    // Fall back to the selection path.
  }
  return copyWithSelection(text, host);
}
