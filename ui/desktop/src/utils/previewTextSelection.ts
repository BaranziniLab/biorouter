import { hasInvalidQuoteUnicode, INVALID_QUOTE_UNICODE, MAX_QUOTE_CHARS } from './quotedText';

export type SelectedText = string | { error: string };
export const PREVIEW_SELECTION_MESSAGE_TYPE = 'biorouter-preview-text-selection';
export const PREVIEW_SELECTION_INSTALL = `(() => {
  if (window.parent === window) return;
  document.addEventListener('selectionchange', () => {
    const text = window.getSelection()?.toString() || '';
    window.parent.postMessage({ type: '${PREVIEW_SELECTION_MESSAGE_TYPE}', text: text.length <= ${MAX_QUOTE_CHARS} ? text : null, length: text.length }, '*');
  });
})()`;

/**
 * What this authenticates, and what it cannot.
 *
 * It authenticates the SENDER — the message must come from a live
 * `biorouter-artifact-preview` frame under `root`. It does NOT authenticate the
 * CLAIM: `data.text` is the frame's word for "what the user selected", and the
 * frame is an opaque-origin sandbox (`allow-scripts allow-downloads`, no
 * `allow-same-origin`), so the host cannot read its selection to check. Script
 * inside an artifact — whose content can include data the agent fetched from
 * somewhere else — can therefore report a selection the user never made, or one
 * whose text differs from what is on screen.
 *
 * ⚠ What makes that safe is NOT this function. It is that attaching requires a
 * host UI action AND that `QuotedTextChip` renders the quotation's full text, so
 * the person sees exactly what they are about to send before they send it. That
 * display is load-bearing, not decoration: it is pinned by
 * `ChatInput.references.test.tsx` asserting the chip's `textContent` contains
 * the quoted text. Shrinking the chip to a label would turn an unverified claim
 * into an invisible one.
 */
export function previewSelectionFromMessage(
  root: HTMLElement,
  event: MessageEvent
): SelectedText | null {
  if (
    !event.source ||
    !Array.from(
      root.querySelectorAll<HTMLIFrameElement>('iframe[name="biorouter-artifact-preview"]')
    ).some((frame) => frame.contentWindow === event.source)
  )
    return null;
  const data = event.data;
  if (
    !data ||
    data.type !== PREVIEW_SELECTION_MESSAGE_TYPE ||
    !Number.isSafeInteger(data.length) ||
    data.length < 0
  )
    return null;
  if (data.text === null && data.length > MAX_QUOTE_CHARS)
    return {
      error: `Select at most ${MAX_QUOTE_CHARS.toLocaleString()} characters; this selection has ${data.length.toLocaleString()}.`,
    };
  if (typeof data.text === 'string' && hasInvalidQuoteUnicode(data.text)) {
    return { error: INVALID_QUOTE_UNICODE };
  }
  return typeof data.text === 'string' &&
    data.text.length === data.length &&
    data.length <= MAX_QUOTE_CHARS
    ? data.text
    : null;
}

export function hasSelectedText(selection: SelectedText): boolean {
  return typeof selection !== 'string' || !!selection.trim();
}
