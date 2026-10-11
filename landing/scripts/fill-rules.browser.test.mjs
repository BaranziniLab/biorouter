// Fills before lines, measured in a real browser (owner message 13, R-27).
//
//     node --test landing/scripts/fill-rules.browser.test.mjs
//
// look-rules.test.mjs reads the stylesheets as text. This reads what the page
// PAINTS, because a rule can be written correctly and still lose the cascade,
// and because the animations build most of their boxes from script. It asserts,
// at 1440 and 390 wide:
//   - a card, a field, a button, a tag and a note rests on the subtle fill with no
//     visible edge, and steps to the strong fill under the pointer;
//   - a chip or field INSIDE a filled box rests on the page ground (a fill never
//     sits on a fill);
//   - the composer cards the animations draw are the one box the fill rule does not
//     reach (owner message 17, R-32): the page ground with the app's resting hairline,
//     and the edge turns coral where the animation shows the box focused;
//   - the art frame has no border and a shadow, the Slack link is a wash with no
//     outline, and no box on any page still draws a visible outline except the
//     floating surfaces and Crew's own recipes;
//   - forced colours draw the outlines the fills dropped;
//   - no page scrolls sideways.
//
// Playwright is resolved out of ui/desktop/node_modules, as the other browser
// tests here do. LANDING_ROOT points the test at another copy of the site, which
// is how it was shown red against the pre-sweep stylesheets.

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

const FILL = 'rgb(244, 244, 242)';
const FILL_STRONG = 'rgb(234, 234, 230)';
const PAPER = 'rgb(255, 255, 255)';
const CORAL = 'rgb(184, 90, 50)';
// --line-composer: --border-subtle at 60%, the resting hairline of the app's composer card (R-32).
const HAIRLINE = 'rgba(228, 228, 224, 0.6)';
const CLEAR = /^(transparent|rgba\(\d+, \d+, \d+, 0\))$/;

// What may still draw a full outline: a floating surface the animations show, the
// schedule's ring marker, Crew's recipes (the app has not converted Crew), and the
// composer cards (R-32: the page ground with a resting hairline).
const OUTLINE_OK = /hub-pick|wall-pop|sk-picker|apps-ask|sch-dot|crew-|apps-go|is-moving|hub-card|wall-card|sk-composer/;

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
  async function open(path, width, extra = {}) {
    const ctx = await browser.newContext({ viewport: { width, height: 900 }, reducedMotion: 'reduce', ...extra });
    const page = await ctx.newPage();
    await page.goto(`${BASE}/${path}`);
    await page.addStyleTag({ content: 'html{scroll-behavior:auto !important}' });
    await page.evaluate(() => document.querySelectorAll('.reveal').forEach((e) => e.classList.add('visible')));
    return { page, close: () => ctx.close() };
  }

  /** The painted styles of the first element matching `selector`. */
  const paint = (page, selector) =>
    page.locator(selector).first().evaluate((e) => {
      const cs = getComputedStyle(e);
      const edge = (s) => ({ w: parseFloat(cs[`border${s}Width`]), c: cs[`border${s}Color`], st: cs[`border${s}Style`] });
      return {
        bg: cs.backgroundColor,
        top: edge('Top'), right: edge('Right'), bottom: edge('Bottom'), left: edge('Left'),
        shadow: cs.boxShadow,
        color: cs.color,
      };
    });
  /** Hovers and focus ease over 125ms: read the painted value once it has landed. */
  const settle = (page) => page.waitForTimeout(260);
  const quiet = (edge) => edge.w === 0 || edge.st === 'none' || CLEAR.test(edge.c);
  const noEdge = (p, what) => {
    for (const side of ['top', 'right', 'bottom', 'left']) assert.ok(quiet(p[side]), `${what}: a visible ${side} edge ${JSON.stringify(p[side])}`);
  };

  for (const width of [1440, 390]) {
    test(`the shared parts rest on fills with no outline at ${width}px`, async () => {
      const { page, close } = await open('baam.html', width);
      await page.waitForSelector('#ext-featured .ext-card');

      const slack = await paint(page, '.header-slack, .nav .slack-link');
      noEdge(slack, 'the Slack link');
      assert.ok(!CLEAR.test(slack.bg), 'the Slack link is a wash');

      const search = await paint(page, '#baam-search');
      assert.equal(search.bg, FILL, 'the BAAM search rests on the fill');
      noEdge(search, 'the BAAM search');
      await page.hover('#baam-search');
      await settle(page);
      assert.equal((await paint(page, '#baam-search')).bg, FILL_STRONG, 'the search steps to the strong fill under the pointer');
      await page.focus('#baam-search');
      await settle(page);
      const focused = await paint(page, '#baam-search');
      assert.equal(focused.top.c, CORAL, 'a typed-in search draws the coral edge');
      assert.equal(focused.bg, FILL, 'and a focused field does not change under the pointer');

      const tag = await paint(page, '#ext-featured .tag:not(.private):not(.public):not(.affiliation)');
      assert.equal(tag.bg, FILL, 'a tag is the fill');
      noEdge(tag, 'a tag');

      const dl = await paint(page, '#ext-featured .brxt-chip');
      assert.equal(dl.bg, FILL, 'the .brxt button is the fill');
      noEdge(dl, 'the .brxt button');
      if (width > 640) {
        await page.hover('#ext-featured .brxt-chip');
        await settle(page);
        const hover = await paint(page, '#ext-featured .brxt-chip');
        assert.equal(hover.bg, FILL_STRONG, 'it steps to the strong fill');
        assert.notEqual(hover.color, CORAL, 'a hover is neutral, never coral');
      }

      const band = await paint(page, '.community');
      assert.equal(band.bg, FILL, 'the Community band is a filled card');
      noEdge(band, 'the Community band');
      await close();
    });

    test(`the docs parts rest on fills with no outline at ${width}px`, async () => {
      const { page, close } = await open('docs.html#privacy', width);
      await page.waitForSelector('.doc-page.active .note');

      const note = await paint(page, '.doc-page.active .note:not(.warn)');
      assert.equal(note.bg, FILL, 'a note is the fill');
      noEdge(note, 'a note');
      const warn = await paint(page, '.doc-page.active .note.warn');
      assert.ok(!CLEAR.test(warn.bg) && warn.bg !== FILL, 'a warning is a wash of the danger hue');
      noEdge(warn, 'a warning');

      const half = await paint(page, '.doc-page.active .split > div');
      assert.equal(half.bg, FILL, 'a half of the split is the fill');
      noEdge(half, 'the split');
      const node = await paint(page, '.doc-page.active .node');
      assert.equal(node.bg, FILL, 'a diagram box is the fill');
      noEdge(node, 'a diagram box');

      // A chip inside a filled box rests on the page ground.
      const chip = await paint(page, '.doc-page.active .split .privacy.is-private');
      assert.equal(chip.bg, PAPER, 'the private pill inside the split rests on the page ground');

      if (width > 880) {
        const search = await paint(page, '.docs-search input');
        assert.equal(search.bg, FILL, 'the docs search rests on the fill');
        noEdge(search, 'the docs search');
        await page.focus('.docs-search input');
        await settle(page);
        assert.equal((await paint(page, '.docs-search input')).top.c, CORAL);
      }
      await close();
    });

    test(`the index draws windows on fills and its composer cards on the page ground at ${width}px`, async () => {
      const { page, close } = await open('index.html', width);
      await page.waitForSelector('.art-hub .hub-card');
      const frame = await paint(page, '.art-hub');
      noEdge(frame, 'the art frame');
      assert.notEqual(frame.shadow, 'none', 'the art frame is lifted by a shadow');
      assert.equal((await paint(page, '.art-hub .hub-mark')).bg, FILL, "the mark's tile is the fill");

      // R-32: a composer card is the page ground with the resting hairline on all four
      // sides, and what sits in it rests on the subtle fill as on any page ground.
      const edgeOf = (p) => ['top', 'right', 'bottom', 'left'].map((s) => `${p[s].w}px ${p[s].st} ${p[s].c}`);
      const hairline = (p, what) => {
        assert.equal(p.bg, PAPER, `${what} is the page ground, not a fill`);
        assert.deepEqual(edgeOf(p), Array(4).fill(`1px solid ${HAIRLINE}`), `${what} draws the resting hairline`);
      };
      hairline(await paint(page, '.art-hub .hub-card'), 'the hub composer card');
      assert.equal((await paint(page, '.art-hub .hub-send')).bg, FILL, 'the idle Send in the card rests on the subtle fill');

      await page.waitForSelector('.art-skills .sk-composer', { state: 'attached' });
      hairline(await paint(page, '.art-skills .sk-composer'), 'the skills composer card');
      assert.equal((await paint(page, '.art-skills .sk-send')).bg, FILL, 'the skills Send rests on the subtle fill');

      await page.waitForSelector('.art-wall .wall-card', { state: 'attached' });
      for (const [i, card] of (await page.locator('.art-wall .wall-card').all()).entries()) {
        const p = await card.evaluate((e) => {
          const cs = getComputedStyle(e);
          const edge = (s) => ({ w: parseFloat(cs[`border${s}Width`]), c: cs[`border${s}Color`], st: cs[`border${s}Style`] });
          return { bg: cs.backgroundColor, top: edge('Top'), right: edge('Right'), bottom: edge('Bottom'), left: edge('Left') };
        });
        hairline(p, `the wall's composer card ${i + 1}`);
      }
      assert.equal((await paint(page, '.art-wall .wall-send')).bg, FILL, "the wall's Send rests on the subtle fill");
      await close();
    });

    test(`a composer card's edge turns coral where the animation shows it focused, and nothing moves, at ${width}px`, async () => {
      const { page, close } = await open('index.html', width);
      await page.waitForSelector('.art-hub .hub-card');
      await page.waitForSelector('.art-skills .sk-composer', { state: 'attached' });
      const box = (sel) => page.locator(sel).first().evaluate((e) => {
        const r = e.getBoundingClientRect();
        return [r.x, r.y, r.width, r.height].map((n) => Math.round(n * 100) / 100);
      });
      const top = async (sel) => (await paint(page, sel)).top.c;

      // The hub shows the box focused for as long as the prompt types: the caret is on.
      const hubBefore = await box('.art-hub .hub-card');
      assert.equal(await top('.art-hub .hub-card'), HAIRLINE);
      await page.evaluate(() => document.querySelector('.art-hub .hub-caret').classList.add('on'));
      await settle(page);
      assert.equal(await top('.art-hub .hub-card'), CORAL, 'the hub card takes the focus edge while the prompt types');
      assert.deepEqual(await box('.art-hub .hub-card'), hubBefore, 'the hub card does not move when the edge turns');
      await page.evaluate(() => document.querySelector('.art-hub .hub-caret').classList.remove('on'));
      await settle(page);
      assert.equal(await top('.art-hub .hub-card'), HAIRLINE, 'the hub card returns to its hairline');

      // The skills tile turns its card's edge while the command is typed.
      const skillsBefore = await box('.art-skills .sk-composer');
      assert.equal(await top('.art-skills .sk-composer'), HAIRLINE);
      await page.evaluate(() => document.querySelector('.art-skills').classList.add('is-edge'));
      await settle(page);
      assert.equal(await top('.art-skills .sk-composer'), CORAL, 'the skills card takes the focus edge while typing');
      assert.deepEqual(await box('.art-skills .sk-composer'), skillsBefore, 'the skills card does not move when the edge turns');
      await close();
    });

    test(`no page scrolls sideways at ${width}px`, async () => {
      for (const path of ['index.html', 'download.html', 'about.html', 'baam.html', 'docs.html#privacy']) {
        const { page, close } = await open(path, width);
        const [sw, iw] = await page.evaluate(() => [document.documentElement.scrollWidth, window.innerWidth]);
        assert.ok(sw <= iw, `${path}: scrollWidth ${sw} > ${iw}`);
        await close();
      }
    });
  }

  test('no box on any page still draws an outline, beyond the floating surfaces and Crew', async () => {
    const pages = [
      'index.html', 'download.html', 'about.html', 'baam.html',
      ...['getstarted', 'app', 'cli', 'privacy', 'extensions', 'agentloop', 'architecture', 'crew', 'providers'].map((d) => `docs.html#${d}`),
    ];
    const outlined = new Map();
    for (const path of pages) {
      const { page, close } = await open(path, 1440);
      await page.waitForTimeout(250);
      const hits = await page.evaluate(() => {
        const out = [];
        const visible = (c) => c && !/^(transparent|rgba\(\d+, \d+, \d+, 0\))$/.test(c);
        for (const e of document.querySelectorAll('body *')) {
          const cs = getComputedStyle(e);
          if (cs.display === 'none') continue;
          const r = e.getBoundingClientRect();
          if (!r.width || !r.height) continue;
          const sides = ['Top', 'Right', 'Bottom', 'Left'].filter(
            (s) => parseFloat(cs[`border${s}Width`]) > 0 && cs[`border${s}Style`] !== 'none' && visible(cs[`border${s}Color`])
          );
          const ring = /inset 0px 0px 0px 1px/.test(cs.boxShadow);
          if (sides.length >= 3 || ring) {
            const cls = String(e.className && e.className.baseVal !== undefined ? e.className.baseVal : e.className);
            out.push(`${e.tagName.toLowerCase()}.${cls.split(' ').join('.')}`);
          }
        }
        return out;
      });
      for (const h of hits) if (!OUTLINE_OK.test(h) && !outlined.has(h)) outlined.set(h, path);
      await close();
    }
    assert.deepEqual([...outlined].map(([h, p]) => `${h} @ ${p}`), []);
  });

  test('forced colours draw the outlines the fills dropped', async () => {
    const { page, close } = await open('docs.html#privacy', 1440, { forcedColors: 'active' });
    await page.waitForSelector('.doc-page.active .note');
    for (const selector of ['.doc-page.active .note', '.doc-page.active .node', '.doc-page.active .split > div', '.community']) {
      const p = await paint(page, selector);
      for (const side of ['top', 'right', 'bottom', 'left']) assert.ok(p[side].w >= 1, `${selector}: no ${side} edge in forced colours`);
    }
    await close();
    const idx = await open('index.html', 1440, { forcedColors: 'active' });
    assert.ok((await paint(idx.page, '.art-hub')).top.w >= 1, 'the art frame keeps an edge in forced colours');
    await idx.close();
  });
}
