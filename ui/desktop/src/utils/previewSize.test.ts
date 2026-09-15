import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  PREVIEW_SIZE_INSTALL,
  PREVIEW_SIZE_MESSAGE_TYPE,
  withPreviewSizeReporting,
} from './previewSize';

/**
 * The reporter a stacked preview's frame runs to say how tall its document is.
 *
 * jsdom lays nothing out, so the geometry is stubbed; what is asserted is the
 * part that decides whether a short figure can ever get a short sheet: the
 * height comes from where the body's CONTENT ends, not from the frame's viewport.
 * Auto Visualiser's own `reportSize` includes `documentElement.clientHeight`, so a
 * figure could never report less than the frame it had already been given.
 */
describe('withPreviewSizeReporting', () => {
  it('goes first inside an existing <head>', () => {
    const html = '<!doctype html><html><head><title>x</title></head><body></body></html>';
    const out = withPreviewSizeReporting(html);
    expect(out.indexOf('<script>')).toBe(out.indexOf('<head>') + '<head>'.length);
    expect(out).toContain('<title>x</title>');
  });

  it('creates a <head> after <html>, or after the doctype, when there is none', () => {
    expect(withPreviewSizeReporting('<html><body>a</body></html>')).toMatch(
      /^<html><head><script>[\s\S]*<\/script><\/head><body>a<\/body><\/html>$/
    );
    expect(withPreviewSizeReporting('<!doctype html><p>a</p>')).toMatch(
      /^<!doctype html><head><script>[\s\S]*<\/script><\/head><p>a<\/p>$/
    );
  });

  it('parses as a script', () => {
    expect(() => new Function(PREVIEW_SIZE_INSTALL)).not.toThrow();
  });
});

describe('PREVIEW_SIZE_INSTALL', () => {
  const originalParent = Object.getOwnPropertyDescriptor(window, 'parent');

  afterEach(() => {
    if (originalParent) Object.defineProperty(window, 'parent', originalParent);
    delete (window as unknown as Record<symbol, unknown>)[Symbol.for('biorouter.preview.size.v1')];
    document.body.innerHTML = '';
    vi.useRealTimers();
    vi.restoreAllMocks();
  });

  it('reports the bottom of the content, not the viewport it sits in', () => {
    vi.useFakeTimers();
    const postMessage = vi.fn();
    Object.defineProperty(window, 'parent', { configurable: true, value: { postMessage } });
    // A 1000px-tall viewport holding a 156px figure: the body box is stretched to
    // the viewport (quirks mode does exactly this), its child is not.
    document.body.innerHTML = '<div id="figure"></div>';
    const figure = document.getElementById('figure') as HTMLElement;
    vi.spyOn(document.body, 'getBoundingClientRect').mockReturnValue({
      top: 0,
      bottom: 1000,
      height: 1000,
      width: 600,
    } as DOMRect);
    vi.spyOn(figure, 'getBoundingClientRect').mockReturnValue({
      top: 12,
      bottom: 156,
      height: 144,
      width: 576,
    } as DOMRect);

    new Function(PREVIEW_SIZE_INSTALL)();
    vi.advanceTimersByTime(50);

    expect(postMessage).toHaveBeenCalledWith({ type: PREVIEW_SIZE_MESSAGE_TYPE, height: 156 }, '*');
    expect(postMessage).not.toHaveBeenCalledWith(
      expect.objectContaining({ height: 1000 }),
      expect.anything()
    );
  });

  it('does nothing in a top-level document, where nobody frames it', () => {
    vi.useFakeTimers();
    const post = vi.spyOn(window, 'postMessage');
    new Function(PREVIEW_SIZE_INSTALL)();
    vi.advanceTimersByTime(50);
    expect(post).not.toHaveBeenCalled();
  });
});
