import { afterEach, describe, expect, it, vi } from 'vitest';
import { PREVIEW_CENTRING_INSTALL } from './previewCentring';
import { withPreviewSizeReporting } from './previewSize';

/**
 * The centring runtime's decisions, with the geometry stubbed.
 *
 * jsdom lays nothing out and computes almost no style, so both are supplied
 * here: what is asserted is the arithmetic and the rules, not the layout. The
 * layout itself is measured in a real Chromium by `previewCentring.browser.test.ts`,
 * which CI's unit job skips for want of a browser, so this file is what holds
 * the logic there.
 */

const DEFAULT_STYLE: Record<string, string> = {
  display: 'block',
  position: 'static',
  overflowX: 'visible',
  overflowY: 'visible',
  transform: 'none',
  translate: 'none',
  backgroundColor: 'rgba(0, 0, 0, 0)',
  backgroundImage: 'none',
  boxShadow: 'none',
  borderLeftWidth: '0px',
  borderRightWidth: '0px',
  borderTopWidth: '0px',
  borderBottomWidth: '0px',
  outlineStyle: 'none',
  outlineWidth: '0px',
};

/** A computed style that is the defaults, plus whatever the element's `data-style` JSON says. */
function stubStyles() {
  vi.spyOn(window, 'getComputedStyle').mockImplementation((el: Element) => {
    const own = JSON.parse((el as HTMLElement).dataset?.style ?? '{}') as Record<string, string>;
    const inline = (el as HTMLElement).style?.translate;
    return { ...DEFAULT_STYLE, ...own, ...(inline ? { translate: inline } : {}) } as never;
  });
}

/** A box that draws something, so the runtime counts it as content rather than looking through it. */
const PAINTED = `data-style='{"backgroundColor":"rgb(255, 255, 255)"}'`;

/** Give `el` a layout box at [left, left + width] in a frame whose visible width is `frame`. */
function place(el: HTMLElement, left: number, width: number) {
  vi.spyOn(el, 'getBoundingClientRect').mockImplementation(() => {
    const shift = Number.parseFloat(el.style.translate) || 0;
    return {
      left: left + shift,
      right: left + width + shift,
      width,
      top: 0,
      bottom: 100,
      height: 100,
    } as DOMRect;
  });
}

function frameOf(width: number) {
  const root = document.documentElement;
  Object.defineProperty(root, 'clientWidth', { configurable: true, value: width });
  Object.defineProperty(root, 'scrollWidth', { configurable: true, value: width });
  Object.defineProperty(window, 'innerWidth', { configurable: true, value: width });
}

function install() {
  new Function('MutationObserver', 'ResizeObserver', PREVIEW_CENTRING_INSTALL)(
    undefined,
    undefined
  );
}

describe('PREVIEW_CENTRING_INSTALL', () => {
  const originalParent = Object.getOwnPropertyDescriptor(window, 'parent');
  const originalInnerWidth = Object.getOwnPropertyDescriptor(window, 'innerWidth');

  afterEach(() => {
    if (originalParent) Object.defineProperty(window, 'parent', originalParent);
    if (originalInnerWidth) Object.defineProperty(window, 'innerWidth', originalInnerWidth);
    delete (document.documentElement as unknown as Record<string, unknown>).clientWidth;
    delete (document.documentElement as unknown as Record<string, unknown>).scrollWidth;
    delete (window as unknown as Record<symbol, unknown>)[
      Symbol.for('biorouter.preview.centre.v1')
    ];
    document.body.innerHTML = '';
    vi.restoreAllMocks();
  });

  const framed = () =>
    Object.defineProperty(window, 'parent', {
      configurable: true,
      value: { postMessage: vi.fn() },
    });

  it('parses as a script and rides in the panel frame script', () => {
    expect(() => new Function(PREVIEW_CENTRING_INSTALL)).not.toThrow();
    expect(withPreviewSizeReporting('<html><head></head><body></body></html>')).toContain(
      PREVIEW_CENTRING_INSTALL
    );
  });

  it('moves a fixed-width canvas flush against the left edge to the middle of the frame', () => {
    framed();
    stubStyles();
    frameOf(620);
    document.body.innerHTML = `<div id="slide" ${PAINTED}></div>`;
    const slide = document.getElementById('slide') as HTMLElement;
    place(slide, 0, 300);

    install();

    // 0 px left and 320 px right become 160 and 160.
    expect(slide.style.translate).toBe('160px');
  });

  it('looks through an unpainted wrapper and moves what is inside it as one group', () => {
    framed();
    stubStyles();
    frameOf(620);
    document.body.innerHTML = `<div id="app"><div id="title">Title</div><div id="figure" ${PAINTED}></div></div>`;
    const title = document.getElementById('title') as HTMLElement;
    const figure = document.getElementById('figure') as HTMLElement;
    place(title, 8, 200);
    place(figure, 8, 300);

    install();

    // Content spans 8..308: 8 px left, 312 px right, so both move 152 px and keep their alignment.
    expect(title.style.translate).toBe('152px');
    expect(figure.style.translate).toBe('152px');
  });

  it('stops at a wrapper that paints, and treats it as the content', () => {
    framed();
    stubStyles();
    frameOf(620);
    document.body.innerHTML = `<div id="page" ${PAINTED}><div id="inner" ${PAINTED}></div></div>`;
    const pageBox = document.getElementById('page') as HTMLElement;
    const inner = document.getElementById('inner') as HTMLElement;
    place(pageBox, 0, 620);
    place(inner, 0, 300);

    install();

    expect(pageBox.style.translate).toBe('');
    expect(inner.style.translate).toBe('');
  });

  it('leaves content alone when it is wider than the frame', () => {
    framed();
    stubStyles();
    frameOf(620);
    Object.defineProperty(document.documentElement, 'scrollWidth', {
      configurable: true,
      value: 968,
    });
    document.body.innerHTML = `<div id="slide" ${PAINTED}></div>`;
    const slide = document.getElementById('slide') as HTMLElement;
    place(slide, 8, 960);

    install();

    expect(slide.style.translate).toBe('');
  });

  it('leaves text that sits directly in the body alone', () => {
    framed();
    stubStyles();
    frameOf(620);
    document.body.innerHTML =
      'Loose text <canvas id="c" data-style=\'{"display":"inline"}\'></canvas>';
    const canvas = document.getElementById('c') as HTMLElement;
    place(canvas, 70, 200);
    place(document.body, 0, 620);

    install();

    expect(canvas.style.translate).toBe('');
  });

  it('does nothing in a top-level document, where nobody frames it', () => {
    stubStyles();
    frameOf(620);
    document.body.innerHTML = `<div id="slide" ${PAINTED}></div>`;
    const slide = document.getElementById('slide') as HTMLElement;
    place(slide, 0, 300);

    install();

    expect(slide.style.translate).toBe('');
  });
});
