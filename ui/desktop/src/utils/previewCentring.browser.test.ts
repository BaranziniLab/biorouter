// @vitest-environment node
/**
 * Whether a previewed HTML document really ends up centred in the panel's frame.
 *
 * jsdom lays nothing out, so this runs the panel's real frame runtime
 * (`withPreviewSizeReporting`, which carries `PREVIEW_CENTRING_INSTALL`) inside a
 * sandboxed `srcdoc` iframe in Chromium, exactly as `ArtifactViewer` frames an
 * HTML artifact, and measures the content against the frame's edges.
 *
 * The first case is the one live verification failed on: a 300x170 canvas with
 * `body { margin: 0 }` sat at 0 px from the left and 59 to 619 px from the right
 * at every panel width. The rest pin what the runtime must leave alone.
 */
import { chromium, type Browser, type Frame, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import { withPreviewSizeReporting } from './previewSize';

/**
 * Launch whichever Chromium this machine has: Playwright's own builds, or an
 * installed Chrome. Scrollbars stay on (Playwright hides them in headless mode by
 * default), because one case measures centring beside a classic scrollbar.
 */
const launchChromium = async (): Promise<Browser | null> => {
  for (const options of [{}, { channel: 'chromium' }, { channel: 'chrome' }] as const) {
    try {
      return await chromium.launch({ ...options, ignoreDefaultArgs: ['--hide-scrollbars'] });
    } catch {
      // try the next build
    }
  }
  return null;
};

const page = (body: string, style = '') =>
  `<!doctype html><html><head><meta charset="utf-8"><style>${style}</style></head><body>${body}</body></html>`;

const SLIDE_STYLE =
  '.slide{width:300px;height:170px;background:#fff;position:relative}' +
  '.l{position:absolute;left:0;top:0;bottom:0;width:12px;background:#e03a3a}';
const SLIDE = '<div class="slide" id="slide"><div class="l"></div><p>Fixed canvas</p></div>';

interface Measure {
  left: number;
  right: number;
  frameRight: number;
  width: number;
  visible: number;
  scrollWidth: number;
  translate: string;
}

/** Where `selector` sits in the frame: gap to the left edge, and to the visible and outer right edges. */
const measure = (frame: Frame, selector: string): Promise<Measure> =>
  frame.evaluate((sel) => {
    const el = document.querySelector(sel) as HTMLElement;
    const rect = el.getBoundingClientRect();
    const root = document.documentElement;
    return {
      left: rect.left,
      right: root.clientWidth - rect.right,
      frameRight: window.innerWidth - rect.right,
      width: rect.width,
      visible: root.clientWidth,
      scrollWidth: root.scrollWidth,
      translate: el.style.translate,
    };
  }, selector);

describe('an HTML preview narrower than its frame', () => {
  let browser: Browser | null = null;

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      // Say so out loud: a silently skipped browser test reads as a passing one.
      console.warn(
        'No Chromium found — skipping the preview centring layout tests. ' +
          'Run `npx playwright install chromium` to enable them.'
      );
    }
  }, 120_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  /** Frame `html` the way the panel does, `width` px wide, and wait for the runtime to settle. */
  const framed = async (html: string, width: number): Promise<{ page: Page; frame: Frame }> => {
    const host = await browser!.newPage({ viewport: { width: 1200, height: 900 } });
    await host.setContent('<!doctype html><html><body style="margin:0"></body></html>');
    await host.evaluate(
      ({ srcdoc, width }) =>
        new Promise<void>((resolve) => {
          const frame = document.createElement('iframe');
          frame.name = 'biorouter-artifact-preview';
          frame.setAttribute('sandbox', 'allow-scripts allow-downloads');
          frame.style.cssText = `width:${width}px;height:600px;border:0;display:block`;
          frame.addEventListener('load', () => resolve(), { once: true });
          frame.srcdoc = srcdoc;
          document.body.append(frame);
        }),
      { srcdoc: withPreviewSizeReporting(html), width }
    );
    await host.waitForTimeout(120);
    const frame = host.frames().find((candidate) => candidate !== host.mainFrame());
    if (!frame) throw new Error('the preview frame never attached');
    return { page: host, frame };
  };

  const resize = async (host: Page, width: number) => {
    await host.evaluate((w) => {
      (document.querySelector('iframe') as HTMLIFrameElement).style.width = `${w}px`;
    }, width);
    await host.waitForTimeout(120);
  };

  it.each([360, 480, 620, 780, 920])(
    'centres a fixed 300px canvas with no body margin in a %ipx frame',
    async (width) => {
      if (!browser) return;
      const { page: host, frame } = await framed(
        page(SLIDE, `body{margin:0;background:#e9ecf3}${SLIDE_STYLE}`),
        width
      );
      const at = await measure(frame, '#slide');
      expect(at.width).toBe(300);
      expect(Math.abs(at.left - at.frameRight)).toBeLessThanOrEqual(1);
      expect(at.left).toBeGreaterThan(20);
      await host.close();
    }
  );

  it('keeps it centred as the frame is dragged wider and back narrower', async () => {
    if (!browser) return;
    const { page: host, frame } = await framed(page(SLIDE, `body{margin:0}${SLIDE_STYLE}`), 360);
    for (const width of [920, 480, 780, 360]) {
      await resize(host, width);
      const at = await measure(frame, '#slide');
      expect(Math.abs(at.left - at.frameRight), `at ${width}px`).toBeLessThanOrEqual(1);
      expect(at.scrollWidth, `no sideways scroll at ${width}px`).toBe(at.visible);
    }
    await host.close();
  });

  it('centres a canvas sitting directly in a body with the default 8px margin', async () => {
    if (!browser) return;
    const { page: host, frame } = await framed(
      page('<canvas id="c" width="300" height="150"></canvas>'),
      620
    );
    const at = await measure(frame, '#c');
    expect(Math.abs(at.left - at.frameRight)).toBeLessThanOrEqual(1);
    await host.close();
  });

  it('looks through an unpainted app root to the canvas inside it', async () => {
    if (!browser) return;
    const { page: host, frame } = await framed(
      page(`<div id="app"><main>${SLIDE}</main></div>`, `body{margin:0}${SLIDE_STYLE}`),
      620
    );
    const at = await measure(frame, '#slide');
    expect(Math.abs(at.left - at.frameRight)).toBeLessThanOrEqual(1);
    expect(at.left).toBeGreaterThan(100);
    await host.close();
  });

  it('centres a box sized in percent without it shrinking or drifting', async () => {
    if (!browser) return;
    const { page: host, frame } = await framed(
      page('<div id="half" style="width:50%;height:80px;background:#ccc"></div>', 'body{margin:0}'),
      600
    );
    const first = await measure(frame, '#half');
    expect(first.width).toBe(300);
    expect(Math.abs(first.left - first.frameRight)).toBeLessThanOrEqual(1);
    await host.waitForTimeout(200);
    expect(await measure(frame, '#half')).toEqual(first);
    await host.close();
  });

  it('centres a canvas added after load', async () => {
    if (!browser) return;
    const { page: host, frame } = await framed(page('', 'body{margin:0}'), 620);
    await frame.evaluate(() => {
      const canvas = document.createElement('canvas');
      canvas.id = 'late';
      canvas.width = 200;
      canvas.height = 100;
      document.body.append(canvas);
    });
    await host.waitForTimeout(120);
    const at = await measure(frame, '#late');
    expect(Math.abs(at.left - at.frameRight)).toBeLessThanOrEqual(1);
    await host.close();
  });

  it('centres across the whole frame and never pushes content under a classic scrollbar', async () => {
    if (!browser) return;
    const { page: host, frame } = await framed(
      page(
        `${SLIDE}<div style="width:300px;height:2000px"></div>`,
        `body{margin:0}${SLIDE_STYLE}::-webkit-scrollbar{width:16px}::-webkit-scrollbar-thumb{background:#999}`
      ),
      620
    );
    const at = await measure(frame, '#slide');
    expect(at.visible).toBe(604);
    expect(Math.abs(at.left - at.frameRight)).toBeLessThanOrEqual(1);
    expect(at.right).toBeGreaterThanOrEqual(0);
    await host.close();
  });

  it('leaves a fixed navigation bar where the author put it', async () => {
    if (!browser) return;
    const { page: host, frame } = await framed(
      page(
        `${SLIDE}<nav id="nav" style="position:fixed;right:20px;bottom:20px;width:80px;height:30px"></nav>`,
        `body{margin:0}${SLIDE_STYLE}`
      ),
      620
    );
    const slide = await measure(frame, '#slide');
    const nav = await measure(frame, '#nav');
    expect(Math.abs(slide.left - slide.frameRight)).toBeLessThanOrEqual(1);
    expect(nav.right).toBe(20);
    expect(nav.translate).toBe('');
    await host.close();
  });

  it('leaves a page-positioned tooltip alone while centring the chart', async () => {
    if (!browser) return;
    const { page: host, frame } = await framed(
      page(
        '<svg id="chart" width="320" height="200"></svg>' +
          '<div id="tip" style="position:absolute;left:40px;top:10px;width:60px;height:20px"></div>',
        'body{margin:0}'
      ),
      620
    );
    const chart = await measure(frame, '#chart');
    const tip = await measure(frame, '#tip');
    expect(Math.abs(chart.left - chart.frameRight)).toBeLessThanOrEqual(1);
    expect(tip.left).toBe(40);
    await host.close();
  });

  describe('what it must not move', () => {
    it('ordinary full-width prose', async () => {
      if (!browser) return;
      const { page: host, frame } = await framed(
        page('<h1 id="h">Report</h1><p>Some prose that fills the width of the page.</p>'),
        620
      );
      const at = await measure(frame, '#h');
      expect(at.left).toBe(8);
      expect(at.translate).toBe('');
      await host.close();
    });

    it('a canvas wider than the frame, which scrolls from its own left edge', async () => {
      if (!browser) return;
      const { page: host, frame } = await framed(
        page(
          '<div id="wide" style="width:960px;height:540px;background:#fff"></div>',
          'body{background:#e9ecf3}'
        ),
        620
      );
      const at = await measure(frame, '#wide');
      expect(at.left).toBe(8);
      expect(at.translate).toBe('');
      expect(at.scrollWidth).toBe(968);
      await host.close();
    });

    it('a canvas that becomes wider than the frame after being centred', async () => {
      if (!browser) return;
      const { page: host, frame } = await framed(page(SLIDE, `body{margin:0}${SLIDE_STYLE}`), 920);
      expect((await measure(frame, '#slide')).left).toBeGreaterThan(100);
      await resize(host, 280);
      const at = await measure(frame, '#slide');
      expect(at.left).toBe(0);
      expect(at.translate).toBe('');
      await host.close();
    });

    it('a canvas beside text that sits directly in the body', async () => {
      if (!browser) return;
      const { page: host, frame } = await framed(
        page('Loose text <canvas id="c" width="200" height="100"></canvas>', 'body{margin:0}'),
        620
      );
      const at = await measure(frame, '#c');
      expect(at.translate).toBe('');
      await host.close();
    });

    it('a box whose absolute child is anchored to the page, not to the box', async () => {
      if (!browser) return;
      const { page: host, frame } = await framed(
        page(
          '<div id="box" style="width:300px;height:100px">' +
            '<span id="pin" style="position:absolute;left:10px;top:10px">x</span></div>',
          'body{margin:0}'
        ),
        620
      );
      expect((await measure(frame, '#box')).translate).toBe('');
      expect((await measure(frame, '#pin')).left).toBe(10);
      await host.close();
    });

    it('a box the author already translates', async () => {
      if (!browser) return;
      const { page: host, frame } = await framed(
        page(
          '<div id="box" style="width:300px;height:100px;translate:5px"></div>',
          'body{margin:0}'
        ),
        620
      );
      const at = await measure(frame, '#box');
      expect(at.translate).toBe('5px');
      expect(at.left).toBe(5);
      await host.close();
    });

    it('content the author already centred', async () => {
      if (!browser) return;
      const { page: host, frame } = await framed(
        page(
          '<div id="box" style="width:300px;height:100px;margin:0 auto"></div>',
          'body{margin:0}'
        ),
        620
      );
      const at = await measure(frame, '#box');
      expect(at.translate).toBe('');
      expect(at.left).toBe(160);
      await host.close();
    });
  });
});
