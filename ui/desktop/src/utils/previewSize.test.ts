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

function installReporter(): () => void {
  const observers: Array<MutationObserver | ResizeObserver> = [];
  const timers = new Set<ReturnType<typeof setTimeout>>();
  const windowListeners = vi.spyOn(window, 'addEventListener');
  const documentListeners = vi.spyOn(document, 'addEventListener');
  const createMutationObserver = function (
    callback: ConstructorParameters<typeof MutationObserver>[0]
  ) {
    const observer = new MutationObserver(callback);
    observers.push(observer);
    return observer;
  };
  const createResizeObserver = function (
    callback: ConstructorParameters<typeof ResizeObserver>[0]
  ) {
    const observer = new ResizeObserver(callback);
    observers.push(observer);
    return observer;
  };
  const schedule = (callback: () => void, delay: number) => {
    const timer = setTimeout(() => {
      timers.delete(timer);
      callback();
    }, delay);
    timers.add(timer);
    return timer;
  };

  new Function('MutationObserver', 'ResizeObserver', 'setTimeout', PREVIEW_SIZE_INSTALL)(
    typeof MutationObserver === 'undefined' ? undefined : createMutationObserver,
    typeof ResizeObserver === 'undefined' ? undefined : createResizeObserver,
    schedule
  );
  const removeListeners = [
    ...windowListeners.mock.calls.map(
      ([type, listener, options]) =>
        () =>
          window.removeEventListener(type, listener, options)
    ),
    ...documentListeners.mock.calls.map(
      ([type, listener, options]) =>
        () =>
          document.removeEventListener(type, listener, options)
    ),
  ];
  windowListeners.mockRestore();
  documentListeners.mockRestore();

  // A browser destroys these with its iframe. This test shares its document with Vitest.
  return () => {
    removeListeners.forEach((remove) => remove());
    observers.forEach((observer) => observer.disconnect());
    timers.forEach((timer) => clearTimeout(timer));
    timers.clear();
  };
}

describe('PREVIEW_SIZE_INSTALL', () => {
  const originalParent = Object.getOwnPropertyDescriptor(window, 'parent');
  let disposeReporter: (() => void) | undefined;

  afterEach(() => {
    disposeReporter?.();
    disposeReporter = undefined;
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

    disposeReporter = installReporter();
    vi.advanceTimersByTime(50);

    expect(postMessage).toHaveBeenCalledWith({ type: PREVIEW_SIZE_MESSAGE_TYPE, height: 156 }, '*');
    expect(postMessage).not.toHaveBeenCalledWith(
      expect.objectContaining({ height: 1000 }),
      expect.anything()
    );
  });

  it('releases queued observations, listeners, and timers before restoring the test environment', async () => {
    vi.useFakeTimers();
    const postMessage = vi.fn();
    Object.defineProperty(window, 'parent', { configurable: true, value: { postMessage } });
    document.body.innerHTML = '<div>Preview</div>';
    disposeReporter = installReporter();
    document.dispatchEvent(new Event('DOMContentLoaded'));
    vi.advanceTimersByTime(50);
    postMessage.mockClear();
    document.body.append(document.createElement('span'));

    disposeReporter();
    expect(vi.getTimerCount()).toBe(0);
    vi.useRealTimers();
    const schedule = vi.spyOn(globalThis, 'setTimeout');
    document.body.innerHTML = '';
    document.dispatchEvent(new Event('DOMContentLoaded'));
    window.dispatchEvent(new Event('load'));
    window.dispatchEvent(new Event('resize'));
    await Promise.resolve();

    expect(schedule).not.toHaveBeenCalled();
    expect(postMessage).not.toHaveBeenCalled();
  });

  it('cancels a queued report when its test document is disposed before the first frame', () => {
    vi.useFakeTimers();
    Object.defineProperty(window, 'parent', {
      configurable: true,
      value: { postMessage: vi.fn() },
    });
    disposeReporter = installReporter();
    document.dispatchEvent(new Event('DOMContentLoaded'));
    expect(vi.getTimerCount()).toBeGreaterThan(0);

    disposeReporter();

    expect(vi.getTimerCount()).toBe(0);
  });

  it('does nothing in a top-level document, where nobody frames it', () => {
    vi.useFakeTimers();
    const post = vi.spyOn(window, 'postMessage');
    disposeReporter = installReporter();
    vi.advanceTimersByTime(50);
    expect(post).not.toHaveBeenCalled();
  });
});
