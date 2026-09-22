import { PREVIEW_SELECTION_INSTALL } from './previewTextSelection';
/**
 * How tall a previewed HTML document WANTS to be, told to the panel that frames it.
 *
 * A stacked artifact preview (rung 2 of the yield ladder) is as tall as its
 * content up to half the pane, so a short figure does not sit in a sheet of
 * empty ground. Text content can be measured from the panel's own DOM; a
 * figure cannot, because it runs in a sandboxed `srcdoc` frame with an opaque
 * origin that the panel may not read. So the panel injects this reporter and
 * the frame posts its height.
 *
 * ⚠ **Its INTRINSIC height, never its viewport's.** Auto Visualiser's own
 * `reportSize` (`_common.js`) takes the max with `documentElement.clientHeight`,
 * which is the frame's viewport — so a figure always reported at least the height
 * it had already been given, fit-to-content could only ever keep it where it was,
 * and every stacked figure sat at exactly half the pane with its x-axis cropped.
 * This measures where the body's content actually ends: the lowest bottom edge of
 * the body's children plus the body's own bottom padding, border and margin. A
 * body stretched to the viewport (quirks mode, `height: 100%`) no longer inflates
 * it, because the children are measured, not the body box.
 *
 * Content genuinely sized by the viewport (`height: 100vh`) still reports the
 * viewport. That is stable rather than a ratchet: the panel only ever uses a
 * reported height to make an undragged sheet SHORTER than half, so a report equal
 * to the current frame keeps the sheet where it is.
 *
 * Injected by the panel alone. The standalone window and the open-in-browser
 * page never frame a sheet, and a report there has nobody listening.
 */
export const PREVIEW_SIZE_MESSAGE_TYPE = 'biorouter-preview-size';

export const PREVIEW_SIZE_INSTALL = `(() => {
  if (window.parent === window) return;
  const key = Symbol.for('biorouter.preview.size.v1');
  if (window[key]) return;
  window[key] = true;
  let last = -1;
  let queued = false;
  const px = (value) => Number.parseFloat(value) || 0;
  const measure = () => {
    const body = document.body;
    if (!body) return 0;
    const style = getComputedStyle(body);
    const offset = window.scrollY || 0;
    let bottom = 0;
    for (const child of body.children) {
      const rect = child.getBoundingClientRect();
      if (rect.width === 0 && rect.height === 0) continue;
      bottom = Math.max(bottom, rect.bottom + offset + px(getComputedStyle(child).marginBottom));
    }
    if (bottom === 0) {
      const rect = body.getBoundingClientRect();
      bottom = rect.top + offset + body.scrollHeight;
    }
    return Math.ceil(bottom + px(style.paddingBottom) + px(style.borderBottomWidth) + px(style.marginBottom));
  };
  const report = () => {
    queued = false;
    const height = measure();
    if (height > 0 && height !== last) {
      last = height;
      window.parent.postMessage({ type: '${PREVIEW_SIZE_MESSAGE_TYPE}', height }, '*');
    }
  };
  const queue = () => {
    if (queued) return;
    queued = true;
    setTimeout(report, 16);
  };
  const watch = () => {
    queue();
    const body = document.body;
    if (!body) return;
    const sizes = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(queue);
    const observeChildren = () => {
      if (!sizes) return;
      sizes.observe(body);
      for (const child of body.children) sizes.observe(child);
    };
    observeChildren();
    if (typeof MutationObserver !== 'undefined') {
      new MutationObserver(() => {
        observeChildren();
        queue();
      }).observe(body, { childList: true, subtree: true });
    }
  };
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', watch);
  else watch();
  window.addEventListener('load', queue);
  window.addEventListener('resize', queue);
})()`;

/** Put the reporter at the top of the document, where `withPreviewActivityTracking` puts its own. */
export function withPreviewSizeReporting(html: string): string {
  const script = `<script>${PREVIEW_SIZE_INSTALL};${PREVIEW_SELECTION_INSTALL}</script>`;
  const head = /<head\b[^>]*>/i.exec(html);
  if (head) {
    const end = head.index + head[0].length;
    return `${html.slice(0, end)}${script}${html.slice(end)}`;
  }
  const root = /<html\b[^>]*>/i.exec(html);
  if (root) {
    const end = root.index + root[0].length;
    return `${html.slice(0, end)}<head>${script}</head>${html.slice(end)}`;
  }
  const doctype = /^\s*<!doctype\b[^>]*>/i.exec(html);
  const end = doctype?.[0].length ?? 0;
  return `${html.slice(0, end)}<head>${script}</head>${html.slice(end)}`;
}
