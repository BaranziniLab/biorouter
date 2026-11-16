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
//      extension in every spelling, as the private one does. A tool row is one line, as the
//      app's is (ToolCallWithResponse): a title the chat is too narrow for ends in an ellipsis
//      with the row's chevron after it, instead of wrapping to a lone "Ran" over a second line
//      (vision round 3, V5-R3-1).
//   2. Opening a BAAM row makes it taller (its description unclamps). The row's GitHub link,
//      download button and chevron used to be centred on the row, so they slid down from under
//      the pointer, and the chevron just pressed was not where a second press would land.
//      They sit on the name's line now, so an open row grows downward and nothing under the
//      pointer moves. (At 860px and below the chevron is pinned to the name's line as it
//      always was; the link and download sit in the tag row there and are not asserted.)
//
//   3. The five "Ways to use Biorouter" tiles on download.html (art/access-*.js) are laid out
//      3 + 2 with the second row centred, then 2 + 2 + 1 with the last tile centred, then one
//      column, every tile the same size and the caption links of a row on one line. Inside the
//      tiles no text is cut except by an ellipsis, no line is cut in half where nothing fades it,
//      and the two tool rows (the desktop app's and Jupyter AI's) are one line each.
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
        // The clipped title span (an older tree has the words straight in .wall-tt: measure that).
        const ttx = tool.querySelector('.wall-ttx') || tool.querySelector('.wall-tt');
        const toolText = shown(tool)
          ? Array.from(ttx.children).find((s) => getComputedStyle(s).display !== 'none' && s.textContent.trim())?.textContent ?? ''
          : null;
        // The tool row's title: one line box, the chevron after it, an ellipsis where it is cut.
        let toolRow = null;
        if (shown(tool)) {
          const tt = tool.querySelector('.wall-tt');
          const chev = tool.querySelector('.wall-tchev');
          const lineHeight = parseFloat(getComputedStyle(tool).lineHeight);
          const tr = tool.getBoundingClientRect();
          const xr = ttx.getBoundingClientRect();
          const cr = chev.getBoundingClientRect();
          const pr = pane.getBoundingClientRect();
          const text = document.createRange();
          text.selectNodeContents(ttx);
          toolRow = {
            lineHeight,
            rowHeight: tr.height,
            titleHeight: tt.getBoundingClientRect().height,
            textLineBoxes: new Set(Array.from(text.getClientRects()).map((q) => Math.round(q.top))).size,
            whiteSpace: getComputedStyle(ttx).whiteSpace,
            ellipsis: getComputedStyle(ttx).textOverflow,
            cut: ttx.scrollWidth > ttx.clientWidth + 1,
            chevronShown: shown(chev) && cr.width > 0,
            chevronAfterText: cr.left >= xr.right - 0.5,
            chevronOnTextLine: Math.abs(cr.top + cr.height / 2 - (xr.top + xr.height / 2)) <= 3,
            chevronInPane: cr.right <= pr.right + 0.5,
            rowInPane: tr.left >= pr.left - 0.5 && tr.right <= pr.right + 0.5,
            hasBadge: !!tool.querySelector('.wall-aff') && shown(tool.querySelector('.wall-aff')),
          };
        }
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
          toolRow,
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

  // A tool row never wraps its title: one line box at every width, as the app's row is. Where the
  // chat is too narrow for the words they end in an ellipsis and the chevron stays after them, on
  // the same line, inside the pane. (Before V5-R3-1 the public row at 390px read "Ran" alone on a
  // line over "LiteratureAgent >", a 40px row beside the private chat's 20px.)
  for (const width of [1440, 1024, 768, 640, 560, 480, 414, 390, 375, 360, 340, 320]) {
    test(`the wall's tool rows are one line, truncating where they must, at ${width}px`, async () => {
      const { page, close } = await open('index.html', width);
      await page.locator('[data-art="wall"]').first().scrollIntoViewIfNeeded();
      await page.waitForTimeout(300);
      const w = await wall(page);
      for (const side of ['priv', 'pub']) {
        const row = w[side].toolRow;
        if (!row) continue; // a chat with no room for its tool row leaves it out; the page still shows the other
        assert.ok(row.titleHeight <= row.lineHeight + 1, `${side}: the title is ${row.titleHeight}px tall, more than one ${row.lineHeight}px line`);
        assert.equal(row.textLineBoxes, 1, `${side}: the title's words sit on ${row.textLineBoxes} lines`);
        assert.equal(row.whiteSpace, 'nowrap', `${side}: the title may not wrap`);
        assert.equal(row.ellipsis, 'ellipsis', `${side}: a title that does not fit must end in an ellipsis, like the app's row`);
        assert.ok(row.chevronShown && row.chevronAfterText && row.chevronOnTextLine, `${side}: the row's chevron must follow the title on its line`);
        assert.ok(row.chevronInPane && row.rowInPane, `${side}: the row leaves its chat`);
        // Without the affiliation badge the whole row is the title: one line, whatever the width.
        if (!row.hasBadge) assert.ok(row.rowHeight <= row.lineHeight + 1, `${side}: the row is ${row.rowHeight}px, ${(row.rowHeight / row.lineHeight).toFixed(1)} lines`);
      }
      assert.ok(w.pub.toolRow, 'the public chat keeps its tool row (it has no badge to wrap, so it always fits as one truncated line)');
      await close();
    });
  }

  test("the wall's public tool row is a single line at 390px, not a 'Ran' over 'LiteratureAgent'", async () => {
    const { page, close } = await open('index.html', 390);
    await page.locator('[data-art="wall"]').first().scrollIntoViewIfNeeded();
    await page.waitForTimeout(300);
    const { pub, priv } = await wall(page);
    assert.ok(pub.toolRow.cut, 'the full name does not fit the 390px chat, so the title is cut');
    assert.ok(pub.toolRow.rowHeight <= pub.toolRow.lineHeight + 1);
    // Both chats' titles share one height: the private chat's badge wraps below, the title does not.
    assert.equal(Math.round(pub.toolRow.titleHeight), Math.round(priv.toolRow.titleHeight));
    await close();
  });

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

  /* ── 3. The ways-to-use tiles: the grid, the captions, no cut text ─── */

  const TILE_NAMES = ['access-gui', 'access-tui', 'access-cli', 'access-serve', 'access-jupyter'];

  /** Each tile's box, and its caption link's top, in page coordinates. */
  const waysGrid = (page) =>
    page.evaluate((names) => names.map((name) => {
      const tile = document.querySelector(`#ways [data-art="${name}"]`);
      const link = tile.closest('.feature').querySelector('.text-link');
      const r = tile.getBoundingClientRect();
      const l = link.getBoundingClientRect();
      return { name, x: r.left + scrollX, y: r.top + scrollY, w: r.width, h: r.height, link: l.top + scrollY };
    }), TILE_NAMES);

  for (const [width, shape] of [[1440, '3+2'], [1280, '3+2'], [1024, '3+2'], [980, '2+2+1'], [768, '2+2+1'], [601, '2+2+1'], [600, '1'], [390, '1'], [360, '1']]) {
    test(`the ways-to-use tiles read ${shape}, centred, with each row's links on one line, at ${width}px`, async () => {
      const { page, close } = await open('download.html', width);
      const t = await waysGrid(page);
      const near = (a, b, what) => assert.ok(Math.abs(a - b) <= 1, `${what}: ${a.toFixed(1)} vs ${b.toFixed(1)}`);
      for (const k of t) {
        near(k.w, t[0].w, `${k.name} is as wide as the first tile`);
        near(k.h / k.w, 0.75, `${k.name} is 4:3`);
      }
      const rows = shape.split('+').map(Number);
      let i = 0;
      const rowTops = [];
      for (const n of rows) {
        const row = t.slice(i, i + n);
        for (const k of row) {
          near(k.y, row[0].y, `${k.name} shares its row's top`);
          near(k.link, row[0].link, `${k.name}'s link shares its row's line`);
        }
        rowTops.push(row[0].y);
        i += n;
      }
      for (let r = 1; r < rowTops.length; r++) assert.ok(rowTops[r] > rowTops[r - 1] + t[0].h, 'rows stack in order');
      if (shape === '3+2') {
        const step = t[1].x - t[0].x;
        near(t[3].x, t[0].x + step / 2, 'the fourth tile starts half a tile in, so the second row is centred');
        near(t[4].x, t[3].x + step, 'the fifth tile follows the fourth');
      } else if (shape === '2+2+1') {
        near(t[2].x, t[0].x, 'the third tile starts a row');
        near(t[4].x, t[0].x + (t[1].x - t[0].x) / 2, 'the last tile is centred under the two above');
      } else {
        for (const k of t) near(k.x, t[0].x, `${k.name} is in the one column`);
      }
      await close();
    });
  }

  /** Glyphs in the tiles that are cut: by a side with no ellipsis, or through the line with nothing
      fading it. Each glyph is read on its own, so a space a wrapped line leaves hanging past the
      edge (white-space: pre-wrap) is not mistaken for cut text. Working outward from the glyph,
      the first box that hides it wholly, or with an ellipsis, settles it. */
  const cutText = (page) =>
    page.evaluate((names) => {
      const out = [];
      const shown = (e) => {
        for (let n = e; n; n = n.parentElement) {
          const cs = getComputedStyle(n);
          if (cs.display === 'none' || cs.visibility === 'hidden' || parseFloat(cs.opacity) === 0) return false;
        }
        return true;
      };
      for (const name of names) {
        const tile = document.querySelector(`#ways [data-art="${name}"]`);
        const walker = document.createTreeWalker(tile, NodeFilter.SHOW_TEXT);
        for (let node = walker.nextNode(); node; node = walker.nextNode()) {
          const text = node.textContent;
          if (!text.trim() || !shown(node.parentElement)) continue;
          const range = document.createRange();
          for (let i = 0; i < text.length; i++) {
            if (/\s/.test(text[i])) continue;
            range.setStart(node, i);
            range.setEnd(node, i + 1);
            const q = range.getBoundingClientRect();
            if (q.width < 0.5 || q.height < 0.5) continue;
            for (let a = node.parentElement; a && a !== tile.parentElement; a = a.parentElement) {
              const cs = getComputedStyle(a);
              const cx = cs.overflowX !== 'visible';
              const cy = cs.overflowY !== 'visible';
              if (!cx && !cy) continue;
              const b = a.getBoundingClientRect();
              if (q.right <= b.left + 0.5 || q.left >= b.right - 0.5 || q.bottom <= b.top + 0.5 || q.top >= b.bottom - 0.5) break;
              const where = () => `${name}: "${text.trim().slice(0, 40)}" in .${String(a.className).split(' ')[0]} (glyph ${Math.round(q.left)}..${Math.round(q.right)} x ${Math.round(q.top)}..${Math.round(q.bottom)}, box ${Math.round(b.left)}..${Math.round(b.right)} x ${Math.round(b.top)}..${Math.round(b.bottom)})`;
              if (cx && (q.left < b.left - 0.5 || q.right > b.right + 0.5)) {
                if (cs.textOverflow === 'ellipsis') break;
                out.push(`${where()} is cut at the side with no ellipsis`);
                break;
              }
              if (cy && (q.top < b.top - 0.5 || q.bottom > b.bottom + 0.5)) {
                if (cs.maskImage !== 'none' || cs.webkitMaskImage !== 'none') break;
                out.push(`${where()} is cut through the line, with nothing fading it`);
                break;
              }
            }
          }
        }
      }
      return [...new Set(out.map((m) => m.replace(/ \(glyph[^)]*\)/, '')))].map((m) => out.find((o) => o.replace(/ \(glyph[^)]*\)/, '') === m));
    }, TILE_NAMES);

  /** The desktop app's tool row and Jupyter AI's: one line, the title nowrap with an ellipsis. */
  const toolRows = (page) =>
    page.evaluate(() => {
      const one = (row, title, chev) => {
        const r = row.getBoundingClientRect();
        const lh = parseFloat(getComputedStyle(title).lineHeight);
        const tr = title.getBoundingClientRect();
        const text = document.createRange();
        text.selectNodeContents(title);
        const lines = new Set(Array.from(text.getClientRects()).filter((q) => q.width > 0).map((q) => Math.round(q.top))).size;
        const cs = getComputedStyle(title);
        const out = {
          rowHeight: r.height, lineHeight: lh, titleHeight: tr.height, lines,
          nowrap: cs.whiteSpace === 'nowrap', ellipsis: cs.textOverflow === 'ellipsis',
          cut: title.scrollWidth > title.clientWidth + 1,
        };
        if (chev) {
          const c = chev.getBoundingClientRect();
          out.chevronAfter = c.left >= tr.right - 0.5;
          out.chevronOnLine = Math.abs(c.top + c.height / 2 - (tr.top + tr.height / 2)) <= 3;
          out.chevronInTile = c.right <= row.closest('.art').getBoundingClientRect().right + 0.5;
        }
        return out;
      };
      const gui = document.querySelector('[data-art="access-gui"] .acc-gui-row');
      const jup = document.querySelector('[data-art="access-jupyter"] .ajl-tool');
      return {
        gui: one(gui, gui.querySelector('.acc-gui-r1'), gui.querySelector('.acc-gui-chev')),
        jupyter: one(jup, jup.querySelector('.ajl-tool-t'), null),
      };
    });

  for (const width of [1440, 1280, 1024, 980, 768, 601, 390, 360]) {
    test(`no text in the ways-to-use tiles is cut but by an ellipsis, and their tool rows are one line, at ${width}px`, async () => {
      const { page, close } = await open('download.html', width);
      await page.locator('#ways').scrollIntoViewIfNeeded();
      await page.waitForTimeout(300);
      assert.deepEqual(await cutText(page), []);
      const rows = await toolRows(page);
      for (const [name, row] of Object.entries(rows)) {
        assert.ok(row.titleHeight <= row.lineHeight + 1, `${name}: the tool title is ${row.titleHeight}px tall, more than one ${row.lineHeight}px line`);
        assert.equal(row.lines, 1, `${name}: the tool title's words sit on ${row.lines} lines`);
        assert.ok(row.nowrap && row.ellipsis, `${name}: the tool title may not wrap and ends in an ellipsis where it is cut`);
      }
      assert.ok(rows.gui.chevronAfter && rows.gui.chevronOnLine && rows.gui.chevronInTile, "the desktop app's chevron follows its tool title on the line");
      await close();
    });
  }
}
