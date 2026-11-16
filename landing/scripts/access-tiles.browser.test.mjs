// The five "Ways to use Biorouter" tiles on download.html, measured in a real browser.
//
//     node --test landing/scripts/access-tiles.browser.test.mjs
//
// Each tile (art/access-*.js) builds its DOM once and runs a loop of timeouts while it is on
// screen. What this holds them to:
//   - every tile is mounted exactly once, into its own frame;
//   - a running tile's pending timeouts stay bounded loop after loop, and stopping it (scrolling
//     it out of view) leaves none behind. The loops run on Playwright's fake clock, so three
//     loops take a second rather than half a minute;
//   - with reduced motion, each tile draws its finished frame at once (the answer in, the panel
//     open, the exit codes printed), schedules nothing, and nothing in it moves afterwards, not
//     even a resize settling on load.
//
// Playwright is resolved out of ui/desktop/node_modules, as the other browser tests here do.
// LANDING_ROOT points the test at another copy of the site.

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

const TILES = ['access-gui', 'access-tui', 'access-cli', 'access-serve', 'access-jupyter'];
// The longest loop of the five is 11.3s (access-gui.js).
const LOOP_MS = 11300;

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

// Counts the timeouts each tile still has pending. It wraps whatever setTimeout the page has
// (the fake clock's, once that is installed), and tells a tile's timeouts apart by the script
// that asked for them.
const COUNT_TIMERS = () => {
  const pending = new Map();
  const set = window.setTimeout;
  const clear = window.clearTimeout;
  const owner = () => {
    const m = /art\/(access-[a-z]+)\.js/.exec(new Error().stack || '');
    return m ? m[1] : null;
  };
  window.setTimeout = function (fn, ms, ...rest) {
    const who = owner();
    let id = 0;
    id = set.call(window, function (...args) {
      pending.delete(id);
      if (typeof fn === 'function') return fn.apply(this, args);
    }, ms, ...rest);
    if (who) pending.set(id, who);
    return id;
  };
  window.clearTimeout = function (id) {
    pending.delete(id);
    return clear.call(window, id);
  };
  window.__pendingByTile = () => {
    const out = {};
    for (const who of pending.values()) out[who] = (out[who] || 0) + 1;
    return out;
  };
};

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

  async function open({ width = 1440, reduced = false, clock = false } = {}) {
    const ctx = await browser.newContext({ viewport: { width, height: width <= 500 ? 844 : 900 }, reducedMotion: reduced ? 'reduce' : 'no-preference' });
    // The page asks GitHub for the latest release; the answer changes nothing here.
    await ctx.route('https://api.github.com/**', (r) => r.fulfill({ status: 404, body: '' }));
    const page = await ctx.newPage();
    const errors = [];
    page.on('pageerror', (e) => errors.push(String(e)));
    if (clock) await page.clock.install();
    await page.addInitScript(COUNT_TIMERS);
    await page.goto(`${BASE}/download.html`);
    await page.addStyleTag({ content: 'html{scroll-behavior:auto !important}' });
    await page.evaluate(() => document.querySelectorAll('.reveal').forEach((e) => e.classList.add('visible')));
    return { page, errors, close: () => ctx.close() };
  }

  test('each tile is mounted once, into its own frame', async () => {
    const { page, errors, close } = await open();
    const mounts = await page.evaluate((names) => names.map((name) => {
      const frames = document.querySelectorAll(`#ways [data-art="${name}"]`);
      const el = frames[0];
      return {
        name,
        frames: frames.length,
        mounted: !!(el && el.__mounted),
        children: el ? el.children.length : 0,
        hidden: el && el.firstElementChild ? el.firstElementChild.getAttribute('aria-hidden') : null,
        label: el ? el.getAttribute('role') === 'img' && (el.getAttribute('aria-label') || '').length > 20 : false,
      };
    }), TILES);
    for (const m of mounts) {
      assert.equal(m.frames, 1, `${m.name}: one frame on the page`);
      assert.ok(m.mounted, `${m.name}: mounted (art/${m.name}.js registered and site.js mounted it)`);
      assert.equal(m.children, 1, `${m.name}: one stage inside the frame, not ${m.children}`);
      assert.equal(m.hidden, 'true', `${m.name}: the drawing is hidden from assistive technology; the frame's label carries it`);
      assert.ok(m.label, `${m.name}: the frame is an img with a label`);
    }
    assert.deepEqual(errors, []);
    await close();
  });

  for (const width of [1440, 390]) {
    test(`running tiles keep their pending timeouts bounded and stopping them leaves none, at ${width}px`, async () => {
      const { page, errors, close } = await open({ width, clock: true });
      const counts = [];
      for (const name of TILES) {
        await page.locator(`[data-art="${name}"]`).scrollIntoViewIfNeeded();
        // IntersectionObserver delivers on the next frames, which the fake clock does not hold.
        await page.waitForTimeout(150);
        const seen = [];
        for (let loop = 0; loop < 3; loop++) {
          seen.push((await page.evaluate(() => window.__pendingByTile()))[name] || 0);
          await page.clock.runFor(LOOP_MS);
        }
        seen.push((await page.evaluate(() => window.__pendingByTile()))[name] || 0);
        counts.push([name, seen]);
        assert.ok(seen[0] > 0, `${name}: started when it came on screen`);
        const max = Math.max(...seen);
        assert.ok(seen.every((n) => n <= seen[0] * 2), `${name}: pending timeouts grew across loops ${JSON.stringify(seen)} (max ${max})`);
      }
      // Off screen, every tile stops and clears what it scheduled.
      await page.evaluate(() => window.scrollTo(0, 0));
      await page.waitForTimeout(250);
      await page.clock.runFor(50);
      const left = await page.evaluate(() => window.__pendingByTile());
      assert.deepEqual(left, {}, `timeouts left after the tiles stopped: ${JSON.stringify(left)}; while running: ${JSON.stringify(counts)}`);
      assert.deepEqual(errors, []);
      await close();
    });
  }

  for (const width of [1440, 1024, 601, 390]) {
    test(`with reduced motion each tile draws its finished frame, schedules nothing and holds still, at ${width}px`, async () => {
      const { page, errors, close } = await open({ width, reduced: true });
      await page.locator('#ways').scrollIntoViewIfNeeded();
      await page.waitForTimeout(150);
      const geometry = () => page.evaluate((names) => names.map((name) => {
        const root = document.querySelector(`[data-art="${name}"]`);
        const base = root.getBoundingClientRect();
        return [...root.querySelectorAll('*')].map((e) => {
          const r = e.getBoundingClientRect();
          const cs = getComputedStyle(e);
          return `${[r.x - base.x, r.y - base.y, r.width, r.height].map((n) => Math.round(n * 2) / 2).join(',')}:${cs.opacity}:${cs.transform}`;
        }).join('|') + `#${root.textContent}`;
      }), TILES);
      const first = await geometry();
      await page.waitForTimeout(1500);
      const later = await geometry();
      for (const [i, name] of TILES.entries()) assert.equal(later[i], first[i], `${name}: something moved after the still frame was drawn`);

      assert.deepEqual(await page.evaluate(() => window.__pendingByTile()), {}, 'a reduced-motion tile schedules nothing');

      const state = await page.evaluate(() => {
        const q = (s) => document.querySelector(s);
        const text = (s) => (q(s) ? q(s).textContent.replace(/\s+/g, ' ').trim() : null);
        return {
          guiOpen: q('[data-art="access-gui"] .acc-gui').classList.contains('is-open'),
          guiAnswer: text('[data-art="access-gui"] .acc-gui-ans'),
          guiPlaceholder: text('[data-art="access-gui"] .acc-gui-ph'),
          tuiReply: text('[data-art="access-tui"] .acc-tui-reply'),
          tuiReplyShown: getComputedStyle(q('[data-art="access-tui"] .acc-tui-reply')).display !== 'none',
          tuiStatus: text('[data-art="access-tui"] .acc-tui-status'),
          // One entry per printed row, a wrapped row's lines joined back with a space.
          cli: [...document.querySelectorAll('[data-art="access-cli"] .acli-row')]
            .map((r) => [...r.querySelectorAll('.acli-ln')].map((l) => l.textContent.trim()).join(' '))
            .join('|'),
          serveOpen: q('[data-art="access-serve"] .acs').classList.contains('is-open'),
          serveReply: text('[data-art="access-serve"] .acs-reply'),
          jupyterAnswer: [...document.querySelectorAll('[data-art="access-jupyter"] .ajl-w.on')].map((w) => w.textContent).join(' '),
          jupyterTool: q('[data-art="access-jupyter"] .ajl-tool').getAttribute('data-state'),
        };
      });
      assert.ok(state.guiOpen, 'desktop app: the side panel is open on the figure');
      assert.equal(state.guiAnswer, 'Here is the figure.');
      assert.equal(state.guiPlaceholder, 'Ask a follow-up', 'desktop app: the composer has moved on to a follow-up');
      assert.ok(state.tuiReplyShown, 'terminal chat: the reply is shown');
      assert.equal(state.tuiReply, 'Two genes are on chr6: HLA-DRB1 and TNF.');
      assert.match(state.tuiStatus, /4%/, 'terminal chat: the context meter has moved');
      assert.match(state.cli, /\|Plan checked: 3 steps, no conflicts\.\|\$ echo \$\?\|0\|\$$/, 'command line: the second run, its exit code and the waiting prompt');
      assert.ok(state.serveOpen, 'browser: the window is open');
      assert.equal(state.serveReply, 'The file has 5,000 rows and 14 columns.');
      assert.equal(state.jupyterAnswer, '412 of 5,000 rows have no age.');
      assert.equal(state.jupyterTool, 'done', 'JupyterLab: the tool call is ticked');
      assert.deepEqual(errors, []);
      await close();
    });
  }
}
