// @vitest-environment node
/**
 * Whether a figure actually runs inside the standalone wrapper page — the
 * artifact window, "open in browser", and the headless renderer's blob: tab.
 *
 * jsdom cannot answer this: it neither evaluates CSP nor implements the
 * inheritance rule that caused the bug (a `srcdoc` document enforces its
 * parent's policy list as well as its own). The figure is a real CDN-mode
 * `render_mermaid` document with the real library spliced in; only the network
 * is stubbed, which is also what production does.
 */
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium, type Browser } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import { inlineArtifactCdnAssets } from './artifactCdnAssets';
import { ARTIFACT_WRAPPER_CSP, wrapArtifactForBrowser } from './artifactSecurity';

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(here, '../../../..');

const MERMAID_CDN_URL = 'https://cdn.jsdelivr.net/npm/mermaid@11.17.2/dist/mermaid.min.js';
const VENDORED_MERMAID = resolve(
  repoRoot,
  'crates/biorouter-mcp/src/autovisualiser/templates/assets/mermaid.min.js'
);
const CDN_FIXTURE = resolve(here, '__fixtures__/autovis-mermaid-cdn.html');

/** The policy this page carried before the fix, kept as a negative control. */
const PRE_FIX_WRAPPER_CSP = "default-src 'none'; style-src 'unsafe-inline'; frame-src 'self'";

const fetchVendored = async (url: string): Promise<string> => {
  if (url !== MERMAID_CDN_URL) throw new Error(`unexpected asset request: ${url}`);
  return readFileSync(VENDORED_MERMAID, 'utf-8');
};

/** Launch whichever Chromium build this machine downloaded (full or headless shell). */
const launchChromium = async (): Promise<Browser | null> => {
  for (const options of [{}, { channel: 'chromium' }] as const) {
    try {
      return await chromium.launch(options);
    } catch {
      // try the next build
    }
  }
  return null;
};

describe('a figure inside the standalone wrapper page', () => {
  let browser: Browser | null = null;
  let figure = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      // Say so out loud: a silently skipped browser test reads as a passing one.
      console.warn(
        'No Playwright Chromium build found — skipping the wrapper CSP tests. ' +
          'Run `npx playwright install chromium` to enable them.'
      );
    }
    figure = await inlineArtifactCdnAssets(readFileSync(CDN_FIXTURE, 'utf-8'), fetchVendored);
  }, 120_000);

  // An explicit budget: tearing down a browser that parsed 3.3 MB of library
  // three times has overrun the suite's default 30 s hook timeout on a loaded
  // machine, which fails the file after every test in it has passed.
  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  const openWrapped = async (wrapperCsp: string) => {
    const page = await browser!.newPage();
    await page.route('**/cdn.jsdelivr.net/**', (route) => route.abort());
    await page.setContent(
      wrapArtifactForBrowser(figure).replace(ARTIFACT_WRAPPER_CSP, wrapperCsp),
      { waitUntil: 'load' }
    );
    return page;
  };

  /** The figure runs in a sandboxed, opaque-origin frame; reach it as a frame. */
  const guestOf = (page: Awaited<ReturnType<typeof openWrapped>>) => {
    const guest = page.frames().find((f) => f !== page.mainFrame());
    if (!guest) throw new Error('the wrapper did not create its preview frame');
    return guest;
  };

  it('runs its scripts and draws the diagram', async (ctx) => {
    if (!browser) return ctx.skip();

    const page = await openWrapped(ARTIFACT_WRAPPER_CSP);
    try {
      await expect
        .poll(
          () =>
            guestOf(page).evaluate(
              () => typeof (window as unknown as Record<string, unknown>).mermaid
            ),
          { timeout: 15_000 }
        )
        .toBe('object');
      await guestOf(page).waitForSelector('#mermaidTarget svg', { timeout: 15_000 });
      expect(await guestOf(page).locator('[role="alert"]').count()).toBe(0);
    } finally {
      await page.close();
    }
  }, 60_000);

  it('renders a data: image, which script-src alone does not restore', async (ctx) => {
    if (!browser) return ctx.skip();

    // Measured while diagnosing this: adding only `script-src` to the wrapper
    // left `default-src 'none'` governing images, so a figure that embeds its
    // own assets stayed broken in a way that looks like a different bug.
    const page = await openWrapped(ARTIFACT_WRAPPER_CSP);
    try {
      const decoded = await guestOf(page).evaluate(async () => {
        const img = document.createElement('img');
        img.src = 'data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7';
        document.body.appendChild(img);
        await new Promise((done) => {
          img.onload = done;
          img.onerror = done;
        });
        return img.naturalWidth;
      });

      expect(decoded).toBe(1);
    } finally {
      await page.close();
    }
  }, 60_000);

  it('ran nothing at all under the policy this page used to carry', async (ctx) => {
    if (!browser) return ctx.skip();

    // The shipped defect, kept as a negative control: the guest inherits the
    // wrapper's `default-src 'none'`, so not even the figure's own error card
    // appears — which is why the surface failed silently.
    const page = await openWrapped(PRE_FIX_WRAPPER_CSP);
    try {
      await page.waitForTimeout(2_000);
      const guest = guestOf(page);
      expect(
        await guest.evaluate(() => typeof (window as unknown as Record<string, unknown>).mermaid)
      ).toBe('undefined');
      expect(await guest.locator('#mermaidTarget svg').count()).toBe(0);
      expect(await guest.locator('[role="alert"]').count()).toBe(0);
    } finally {
      await page.close();
    }
  }, 60_000);
});
