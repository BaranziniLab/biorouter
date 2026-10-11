// Two layout promises the site makes, measured in a real browser (vision round 2, V5-R2-1 and V5-R2-2).
//
//     node --test landing/scripts/layout-stability.browser.test.mjs
//
// jsdom computes no layout, and both promises are about layout:
//
//   1. The wall animation draws the app's composer, and the app's model chip never hides the
//      model's name: it truncates it inside the chip (ComposerChips). The mock used to give the
//      name up first in a narrow chat, leaving a lone padlock that reads as a lock button. Now
//      the chip keeps the name and ellipsizes it, and the public chat's tool row names its
//      extension in every spelling, as the private one does.
//   2. Opening a BAAM row makes it taller (its description unclamps). The row's GitHub link,
//      download button and chevron used to be centred on the row, so they slid down from under
//      the pointer, and the chevron just pressed was not where a second press would land.
//      They sit on the name's line now, so an open row grows downward and nothing under the
//      pointer moves. (At 860px and below the chevron is pinned to the name's line as it
//      always was; the link and download sit in the tag row there and are not asserted.)
//
// Playwright is resolved out of ui/desktop/node_modules, as the other browser tests here do.
// LANDING_ROOT points the test at another copy of the site, which is how it was shown red
// against the stylesheets from before the fix.

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { createReadStream, existsSync, statSync } from 'node:fs';
import { dirname, extname, join, normalize, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const SCRIPTS = dirname(fileURLToPath(import.meta.url));
const REPO = resolve(SCRIPTS, '../..');
const ROOT = process.env.LANDING_ROOT ? resolve(process.env.LANDING_ROOT) : resolve(SCRIPTS, '..');
const PLAYWRIGHT = join(REPO, 'ui/desktop/node_modules/playwright/index.mjs');

const TYPES = {
  '.html': 'text/html; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.json': 'application/json; charset=utf-8',
  '.svg': 'image/svg+xml',
  '.png': 'image/png',
};

function serve() {
  const server = createServer((req, res) => {
    const path = decodeURIComponent((req.url || '/').split('?')[0]);
    const file = join(ROOT, normalize(path).replace(/^(\.\.[/\\])+/, ''));
    if (!file.startsWith(ROOT) || !existsSync(file) || !statSync(file).isFile()) {
      res.writeHead(404).end('not found');
      return;
    }
    res.writeHead(200, { 'content-type': TYPES[extname(file)] || 'application/octet-stream' });
    createReadStream(file).pipe(res);
  });
  return new Promise((ok) => server.listen(0, '127.0.0.1', () => ok(server)));
}

async function launch(chromium) {
  const failures = [];
  for (const opts of [{}, { channel: 'chromium' }, { channel: 'chrome' }]) {
    try {
      return await chromium.launch(opts);
    } catch (err) {
      failures.push(`${JSON.stringify(opts)}: ${String(err).split('\n')[0]}`);
    }
  }
  throw new Error(`no chromium could be launched:\n  ${failures.join('\n  ')}`);
}

if (!existsSync(PLAYWRIGHT)) {
  test('playwright is available', () => {
    assert.fail(`${PLAYWRIGHT} is missing: run \`cd ui/desktop && npm ci\`.`);
  });
} else {
  const { chromium } = await import(PLAYWRIGHT);
  const server = await serve();
  const BASE = `http://127.0.0.1:${server.address().port}`;
  const browser = await launch(chromium);
  test.after(async () => {
    await browser.close();
    server.close();
  });

  /** Open a page with motion reduced (the animations draw their final frame) and every reveal shown. */
  async function open(path, width) {
    const ctx = await browser.newContext({ viewport: { width, height: width <= 500 ? 844 : 900 }, reducedMotion: 'reduce' });
    const page = await ctx.newPage();
    await page.goto(`${BASE}/${path}`);
    await page.addStyleTag({ content: 'html{scroll-behavior:auto !important}' });
    await page.evaluate(() => document.querySelectorAll('.reveal').forEach((e) => e.classList.add('visible')));
    return { page, close: () => ctx.close() };
  }

  /* ── 1. The wall's model chip keeps its name ─────────────────────────── */

  /** What each composer's bar and chip show, and what the public tool row says. */
  const wall = (page) =>
    page.evaluate(() => {
      const shown = (e) => !!e && getComputedStyle(e).display !== 'none';
      const side = (name) => {
        const pane = document.querySelector(`.art-wall .wall-${name} .wall-pane`);
        const bar = pane.querySelector('.wall-bar');
        const chip = pane.querySelector('.wall-chip');
        const cn = pane.querySelector('.wall-cn');
        const send = pane.querySelector('.wall-send');
        const tool = pane.querySelector('.wall-m-tool');
        const toolText = shown(tool)
          ? Array.from(tool.querySelectorAll('.wall-tt > span')).find((s) => getComputedStyle(s).display !== 'none')?.textContent ?? ''
          : null;
        return {
          barOverflows: bar.scrollWidth > bar.clientWidth + 1,
          chipShown: shown(chip),
          nameShown: shown(cn),
          name: cn.textContent,
          nameWidth: shown(cn) ? cn.getBoundingClientRect().width : 0,
          nameCut: shown(cn) && cn.scrollWidth > cn.clientWidth + 1,
          ellipsis: getComputedStyle(cn).textOverflow,
          chipInsideBar: shown(chip) && chip.getBoundingClientRect().right <= send.getBoundingClientRect().left + 1,
          toolText,
        };
      };
      return { priv: side('priv'), pub: side('pub') };
    });

  for (const width of [390, 360, 320]) {
    test(`the wall's composers keep the model's name at ${width}px`, async () => {
      const { page, close } = await open('index.html', width);
      await page.locator('[data-art="wall"]').first().scrollIntoViewIfNeeded();
      await page.waitForTimeout(300);
      const w = await wall(page);
      for (const [side, want] of [['priv', 'Versa API Azure'], ['pub', 'Claude']]) {
        const s = w[side];
        assert.ok(!s.barOverflows, `${side}: the controls row overflows its card`);
        assert.ok(s.chipShown && s.nameShown, `${side}: the chip lost its name (a lone padlock reads as a lock button)`);
        assert.equal(s.name, want);
        assert.ok(s.nameWidth >= 24, `${side}: the name is ${s.nameWidth}px wide, too little to read`);
        assert.ok(s.chipInsideBar, `${side}: the chip runs under Send`);
        assert.equal(s.ellipsis, 'ellipsis', `${side}: a name that does not fit must end in an ellipsis, like the app's chip`);
      }
      // The private chat's name does not fit here, so it is cut, not hidden.
      assert.ok(w.priv.nameCut, 'the private chat should ellipsize its longer name at this width');
      await close();
    });
  }

  test("the wall's composers show the whole name where it fits (1440px)", async () => {
    const { page, close } = await open('index.html', 1440);
    await page.locator('[data-art="wall"]').first().scrollIntoViewIfNeeded();
    await page.waitForTimeout(300);
    const w = await wall(page);
    assert.equal(w.priv.name, 'Versa API Azure');
    assert.ok(w.priv.nameShown && !w.priv.nameCut, 'the whole private model name shows at 1440');
    assert.ok(w.pub.nameShown && !w.pub.nameCut, 'the whole public model name shows at 1440');
    await close();
  });

  for (const width of [1440, 390]) {
    test(`the public chat's tool row names its extension at ${width}px`, async () => {
      const { page, close } = await open('index.html', width);
      await page.locator('[data-art="wall"]').first().scrollIntoViewIfNeeded();
      await page.waitForTimeout(300);
      const w = await wall(page);
      assert.ok(w.pub.toolText, 'the public tool row is shown');
      assert.match(w.pub.toolText, /^Ran LiteratureAgent/, 'the tool row names the extension, as the private chat does');
      assert.doesNotMatch(w.pub.toolText, /^Ran PubMed search$/, 'the short spelling does not invent another name');
      await close();
    });
  }

  /* ── 2. A BAAM row's controls stay put when it opens ─────────────────── */

  const SHELVES = ['extensions', 'skills', 'workflows'];
  const ROWS = (shelf) => `#${shelf}-section .ext-card, #${shelf}-section .skill-card, #${shelf}-section .wf-card`;
  const PARTS = '.ext-gh-link, .brxt-chip, .skill-dl-btn, .row-toggle';

  /** Each control's top, relative to its row, and the name's vertical centre. */
  const read = (row) =>
    row.evaluate((card, sel) => {
      const cr = card.getBoundingClientRect();
      const tops = {};
      for (const el of card.querySelectorAll(sel)) {
        if (el.closest('.row-detail') || getComputedStyle(el).display === 'none') continue;
        tops[el.className.split(' ')[0]] = el.getBoundingClientRect().top - cr.top;
      }
      const h = card.querySelector(':scope h3').getBoundingClientRect();
      return { tops, nameMid: h.top + h.height / 2 - cr.top, height: cr.height };
    }, PARTS);

  for (const width of [1440, 1024, 880]) {
    test(`opening a BAAM row moves none of its controls at ${width}px`, async () => {
      const { page, close } = await open('baam.html', width);
      await page.waitForSelector('#ext-featured .ext-card');
      for (const shelf of SHELVES) {
        await page.click(`#tab-${shelf}`);
        await page.waitForTimeout(250);
        const rows = page.locator(ROWS(shelf));
        const n = Math.min(await rows.count(), 4);
        assert.ok(n > 0, `${shelf} has rows`);
        for (let i = 0; i < n; i++) {
          const row = rows.nth(i);
          await row.scrollIntoViewIfNeeded();
          const closed = await read(row);
          assert.ok(closed.tops['row-toggle'] !== undefined, `${shelf}[${i}] has a chevron`);
          await row.locator(':scope > .row-toggle').click();
          await page.waitForTimeout(350);
          const opened = await read(row);
          assert.ok(opened.height > closed.height + 20, `${shelf}[${i}] grew when opened`);
          for (const [name, top] of Object.entries(closed.tops)) {
            assert.ok(Math.abs(opened.tops[name] - top) < 0.5, `${shelf}[${i}] ${name} moved ${top} -> ${opened.tops[name]} when the row opened`);
            // Level with the name, which is also where the glyph and the open row's rail sit.
            assert.ok(Math.abs(top + 16 - closed.nameMid) <= 1.5, `${shelf}[${i}] ${name} is ${(top + 16 - closed.nameMid).toFixed(1)}px off the name's line`);
          }
          await row.locator(':scope > .row-toggle').click();
          await page.waitForTimeout(250);
        }
      }
      await close();
    });
  }

  for (const width of [1440, 390]) {
    test(`the chevron is still under the pointer after it opens a row at ${width}px`, async () => {
      const { page, close } = await open('baam.html', width);
      await page.waitForSelector('#ext-featured .ext-card');
      const row = page.locator(ROWS('extensions')).first();
      await row.scrollIntoViewIfNeeded();
      const toggle = row.locator(':scope > .row-toggle');
      const box = await toggle.boundingBox();
      const x = box.x + box.width / 2;
      const y = box.y + box.height / 2;
      await page.mouse.click(x, y);
      await page.waitForTimeout(350);
      assert.equal(await row.evaluate((e) => e.classList.contains('open')), true, 'the press opened the row');
      // A row's own click closes it wherever it lands, so the row closing again proves nothing:
      // what matters is that the control the person is pointing at has not left them.
      const under = await page.evaluate(([px, py]) => {
        const hit = document.elementFromPoint(px, py);
        return hit ? hit.closest('.row-toggle') !== null : false;
      }, [x, y]);
      assert.ok(under, 'the chevron moved out from under the pointer when the row opened');
      await close();
    });
  }
}
