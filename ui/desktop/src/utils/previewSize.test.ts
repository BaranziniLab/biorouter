import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
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
  type WindowListener = Parameters<typeof window.addEventListener>[1];
  type WindowListenerOptions = Parameters<typeof window.addEventListener>[2];
  type PreviewMutationObserver = InstanceType<typeof window.MutationObserver>;
  type PreviewMutationCallback = ConstructorParameters<typeof window.MutationObserver>[0];
  const installedObservers: PreviewMutationObserver[] = [];
  const installedListeners: Array<{
    type: string;
    listener: WindowListener;
    options?: WindowListenerOptions;
  }> = [];
  const originalAddEventListener = window.addEventListener;
  const originalRemoveEventListener = window.removeEventListener;
  const originalMutationObserver = window.MutationObserver;

  beforeEach(() => {
    window.addEventListener = ((
      type: string,
      listener: WindowListener,
      options?: WindowListenerOptions
    ) => {
      installedListeners.push({ type, listener, options });
      return originalAddEventListener.call(window, type, listener, options);
    }) as typeof window.addEventListener;
    window.MutationObserver = class extends originalMutationObserver {
      constructor(callback: PreviewMutationCallback) {
        super(callback);
        installedObservers.push(this);
      }
    };
  });

  afterEach(() => {
    for (const observer of installedObservers.splice(0)) observer.disconnect();
    for (const { type, listener, options } of installedListeners.splice(0)) {
      originalRemoveEventListener.call(window, type, listener, options);
    }
    vi.clearAllTimers();
    window.addEventListener = originalAddEventListener;
    window.removeEventListener = originalRemoveEventListener;
    window.MutationObserver = originalMutationObserver;
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
