import { MAX_QUOTE_CHARS } from './quotedText';

export type SelectedText = string | { error: string };
export const PREVIEW_SELECTION_MESSAGE_TYPE = 'biorouter-preview-text-selection';
export const PREVIEW_SELECTION_INSTALL = `(() => {
  if (window.parent === window) return;
  document.addEventListener('selectionchange', () => {
    const text = window.getSelection()?.toString() || '';
    window.parent.postMessage({ type: '${PREVIEW_SELECTION_MESSAGE_TYPE}', text: text.length <= ${MAX_QUOTE_CHARS} ? text : null, length: text.length }, '*');
  });
})()`;

// The frame can report source data only. Attaching it always requires a host UI action.
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
  return typeof data.text === 'string' &&
    data.text.length === data.length &&
    data.length <= MAX_QUOTE_CHARS
    ? data.text
    : null;
}

export function hasSelectedText(selection: SelectedText): boolean {
  return typeof selection !== 'string' || !!selection.trim();
}
