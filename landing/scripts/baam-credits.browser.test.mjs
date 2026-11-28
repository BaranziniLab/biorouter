// The BAAM page's credit lines, curator lines and provenance panel, in a real browser.
//
//     node --test landing/scripts/baam-credits.browser.test.mjs
//
// Every card on the BAAM shelves carries one line saying where the item came from
// (`<p class="item-credit">`, written into baam.html by write-credits.mjs and, for the
// rendered extension shelf, by baam.js from credits.json). Each shelf ends with one
// curator line and the footer carries another, and an opened row shows a
// "Provenance and credits" row built from credits.json. PV14 in check-catalog.mjs
// holds the page's TEXT to the catalog; this file holds what a visitor SEES:
//
//   1. In the rendered view, at 1440 and 390 wide, every extension card (featured,
//      primary and the expanded integrations) and every skill card (featured and the
//      three expanded groups) has exactly one line, the next element after its
//      description, reading credits.json's `line`, visible, under the description and
//      over the tag row, inside its card, and the page does not scroll sideways.
//      At 1440, 640 and 390 wide, every skill card's version chip sits between its
//      type line and its description (never under the credit line), opening a row
//      does not move it, and an open row's chips, the longest licence chip
//      included, stay inside the card.
//   2. Without JavaScript, every static card has its line and the extension lines show.
//   3. Each shelf's curator line shows when the shelf is selected, the footer's shows,
//      and no card carries the curator sentence.
//   4. At 1440 and 390 wide, eight named rows open on a provenance row that matches
//      their credits.json entry (summary, the source or idea list with "Show all N",
//      data and earlier versions collapsed, people, curator, Licence files and Full
//      record links), every link in it is http(s), the row stays inside its card
//      with every fold opened and the page does not scroll sideways, and opening the
//      row moves none of the controls beside the name (on a phone an extension's
//      GitHub link and download sit under the description and move only as far as
//      its opened clamp grows).
//   5. Steps 1 and 3 run under a light and a dark colour scheme, and every line paints
//      at 4.5:1 or more against what is painted under it (rasterised, as
//      baam-privacy-facet.test.mjs reads its badges).
//   6. Credit values are text: markup served in a card line and in every kind of
//      value the panel writes (summary, source text, data terms, a linked name, a
//      person), and a javascript: source URL, reach the page as literal characters,
//      create no element, link nothing and run nothing.
//   7. The credit line is not searched: a search for a name that only a credit line
//      carries finds no card, and every tag facet shows the same cards with every line
//      removed (and a facet word planted in one line does not bring its card in).
//   8. With credits.json answering 404, at 1440 and 390 wide, the rendered extension
//      cards keep the lines harvested from the static view and an opened row still
//      names its line and the curator, inside its card.
//
// Every check was first shown failing against a mutant of the page (one per
// behaviour: a line left out, written with innerHTML, too faint, on the tag row or
// unwrapped on a phone; a facet or search that reads it; a panel that drops or
// misplaces a part, or on a phone only is hidden, wider than its card or placed
// above the controls; a fallback that harvests nothing) before it was trusted.
//
// The expected values are read from credits.json in the served folder when the file
// starts, so a catalog change moves the test with it. LANDING_ROOT serves another copy
// of the site instead of landing/, which is how this file was run against the page
// before the credits landed. Playwright is resolved out of ui/desktop/node_modules, as
// the other browser tests here do, and a machine where no chromium launches FAILS
// rather than skips.
//
// Where this runs: `just check-shelf`, the `shelf` job of .github/workflows/frontend.yml
// and the `check` job of .github/workflows/deploy-landing.yml, which gates the Pages
// deploy.

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { createReadStream, existsSync, readFileSync, statSync } from 'node:fs';
import { dirname, extname, join, normalize, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const SCRIPTS = dirname(fileURLToPath(import.meta.url));
const REPO = resolve(SCRIPTS, '../..');
const ROOT = process.env.LANDING_ROOT ? resolve(process.env.LANDING_ROOT) : resolve(SCRIPTS, '..');
const PLAYWRIGHT = join(REPO, 'ui/desktop/node_modules/playwright/index.mjs');
const CREDITS_FILE = join(ROOT, 'credits.json');

const TYPES = {
  '.html': 'text/html; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.mjs': 'text/javascript; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.json': 'application/json; charset=utf-8',
  '.svg': 'image/svg+xml',
  '.png': 'image/png',
};

/** The rows the panel test opens, by kind, with what each one stands for. */
const PANEL_ITEMS = [
  ['extensions', 'cdwagent', 'in-house'],
  ['extensions', 'biomcpagent', 'third-party, with a data list'],
  ['extensions', 'idcagent', 'a bridge to a hosted service'],
  ['extensions', 'opencomputeruseagent', 'adapted, three sources'],
  ['skills', 'replication-package-audit', 'adapted, ten sources'],
  ['skills', 'empirical-research-router', 'ideas-derived'],
  ['skills', 'superpowers', 'a third-party bundle'],
  ['skills', 'scientific-research', 'in-house'],
];
const PANEL_ROW_LABEL = 'Provenance and credits';
const PANEL_SHOWN = 3;
const SHELVES = ['extensions', 'workflows', 'skills'];
const MIN_CONTRAST = 4.5;
const PAYLOAD = '<img src=x onerror="window.__pwned=1">';
const BAD_URL = 'javascript:alert(1)';

function readCredits() {
  if (!existsSync(CREDITS_FILE)) return { error: `${CREDITS_FILE} is missing: run node landing/scripts/build-registry.mjs` };
  try {
    const c = JSON.parse(readFileSync(CREDITS_FILE, 'utf8'));
    if (c.version !== 1 || !c.extensions || !c.skills || typeof c.curated_by !== 'string') {
      return { error: `${CREDITS_FILE} is not credits.json version 1` };
    }
    return { credits: c };
  } catch (err) {
    return { error: `${CREDITS_FILE} does not parse: ${err.message}` };
  }
}

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
  throw new Error(`no chromium could be launched:\n  ${failures.join('\n  ')}\nInstall one with: cd ui/desktop && npx playwright install chromium`);
}

/**
 * Installed in the page as window.__bt. Rasterises colours rather than parsing
 * them: a computed colour can come back in a form a regex misreads, and painting
 * the layers lets the browser do the parsing and the alpha compositing.
 */
function installHelpers() {
  const cv = document.createElement('canvas');
  cv.width = cv.height = 1;
  const g = cv.getContext('2d', { willReadFrequently: true });
  const paint = (layers) => {
    g.clearRect(0, 0, 1, 1);
    g.fillStyle = '#ffffff';
    g.fillRect(0, 0, 1, 1);
    for (const css of layers) {
      // Reset first, so a value the canvas cannot parse paints nothing rather
      // than repainting the layer before it.
      g.fillStyle = 'rgba(0, 0, 0, 0)';
      g.fillStyle = css;
      g.fillRect(0, 0, 1, 1);
    }
    return [...g.getImageData(0, 0, 1, 1).data].slice(0, 3);
  };
  const lum = (rgb) => {
    const f = (c) => {
      const s = c / 255;
      return s <= 0.03928 ? s / 12.92 : Math.pow((s + 0.055) / 1.055, 2.4);
    };
    return 0.2126 * f(rgb[0]) + 0.7152 * f(rgb[1]) + 0.0722 * f(rgb[2]);
  };
  /** The text colour of `el` against everything painted under it, as a WCAG ratio. */
  const contrast = (el) => {
    const chain = [];
    for (let n = el; n; n = n.parentElement) chain.unshift(n);
    const layers = chain.map((n) => getComputedStyle(n).backgroundColor);
    const bg = paint(layers);
    const fg = paint([...layers, getComputedStyle(el).color]);
    const [a, b] = [lum(fg), lum(bg)].sort((x, y) => y - x);
    return Math.round(((a + 0.05) / (b + 0.05)) * 100) / 100;
  };
  const visible = (el) => {
    if (!el || !el.checkVisibility({ opacityProperty: true, visibilityProperty: true })) return false;
    const r = el.getBoundingClientRect();
    return r.width > 0 && r.height > 0;
  };
  const slug = (href) => String(href || '').split('?')[0].split('/').pop().replace(/\.(brxt|zip)$/i, '');
  const kindOf = (card) => (card.classList.contains('skill-card') ? 'skills' : 'extensions');
  const idOf = (card) => {
    if (kindOf(card) === 'skills') return slug(card.querySelector('.skill-dl-btn')?.getAttribute('href'));
    if (card.dataset.registryId) return card.dataset.registryId.trim();
    return slug(card.querySelector('.brxt-chip')?.getAttribute('href'));
  };
  /** A closed row's text without the panel an open row carries. */
  const rowText = (card) => {
    let t = '';
    card.childNodes.forEach((n) => {
      if (n.nodeType === 1 && n.classList.contains('row-detail')) return;
      t += ' ' + n.textContent;
    });
    return t;
  };
  /** Everything step 1 asks of one card. */
  const cardReport = (card, curatedBy) => {
    const desc = card.querySelector(':scope > .ext-desc, :scope > .skill-desc');
    const lines = [...card.querySelectorAll(':scope > .item-credit')];
    const line = lines[0] || null;
    const tags = card.querySelector('.ext-tags, .skill-tags');
    const k = card.getBoundingClientRect();
    const r = line && line.getBoundingClientRect();
    const d = desc && desc.getBoundingClientRect();
    const t = tags && tags.getBoundingClientRect();
    const shown = visible(line);
    return {
      kind: kindOf(card),
      id: idOf(card),
      direct: lines.length,
      all: card.querySelectorAll('.item-credit').length,
      tag: line ? line.tagName.toLowerCase() : null,
      text: line ? line.textContent : null,
      afterDesc: !!line && !!desc && desc.nextElementSibling === line,
      visible: shown,
      cardShown: visible(card),
      belowDesc: !!(r && d) && r.top >= d.bottom - 0.5,
      tagsShown: !!t && t.height > 0,
      aboveTags: !t || t.height === 0 || (!!r && r.bottom <= t.top + 0.5),
      inCard: !!r && r.left >= k.left - 0.5 && r.right <= k.right + 0.5,
      overflows: !!line && line.scrollWidth > line.clientWidth + 1,
      contrast: shown ? contrast(line) : null,
      curator: rowText(card).includes(curatedBy),
    };
  };
  window.__bt = { contrast, visible, idOf, kindOf, cardReport, rowText };
}

/**
 * Where a row's controls sit, relative to the row (layout-stability.browser.test.mjs's
 * reading), and where its description sits: on a phone an extension card's
 * GitHub link and download sit under the description, whose clamp opens with
 * the row, so they move with it there by design.
 */
const CONTROLS = '.ext-gh-link, .brxt-chip, .skill-dl-btn, .row-toggle';
const controlTops = (row) =>
  row.evaluate((card, sel) => {
    const cr = card.getBoundingClientRect();
    const tops = {};
    for (const el of card.querySelectorAll(sel)) {
      if (el.closest('.row-detail') || getComputedStyle(el).display === 'none') continue;
      tops[el.className.split(' ')[0]] = el.getBoundingClientRect().top - cr.top;
    }
    const d = card.querySelector(':scope > .ext-desc, :scope > .skill-desc')?.getBoundingClientRect();
    return { tops, height: cr.height, descTop: d ? d.top - cr.top : null, descHeight: d ? d.height : 0 };
  }, CONTROLS);

const { credits: CREDITS, error: CREDITS_ERROR } = readCredits();

if (!existsSync(PLAYWRIGHT)) {
  test('playwright is available', () => {
    assert.fail(`${PLAYWRIGHT} is missing: run \`cd ui/desktop && npm ci\`.`);
  });
} else if (CREDITS_ERROR) {
  test('credits.json is there to compare against', () => {
    assert.fail(CREDITS_ERROR);
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

  const CURATED_BY = CREDITS.curated_by;
  const active = (kind) => Object.keys(CREDITS[kind]).filter((id) => CREDITS[kind][id].status === 'active');
  const entry = (kind, id) => (Object.hasOwn(CREDITS[kind], id) ? CREDITS[kind][id] : null);

  /**
   * A page on baam.html. `credits` is a route handler for credits.json, if the
   * test answers it itself. With JavaScript on, it waits for the registry to
   * render the extension shelf and, for up to three seconds, for its lines.
   */
  async function open({ width = 1440, scheme = 'light', javaScriptEnabled = true, credits } = {}) {
    const ctx = await browser.newContext({
      viewport: { width, height: width <= 500 ? 844 : 900 },
      colorScheme: scheme,
      reducedMotion: 'reduce',
      javaScriptEnabled,
    });
    // site.js asks the GitHub API for the release number. Answer it here, so the
    // page paints the same with and without a network.
    await ctx.route('https://api.github.com/**', (route) =>
      route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ tag_name: 'v9.9.9' }) }),
    );
    if (credits) await ctx.route('**/credits.json', credits);
    const page = await ctx.newPage();
    const errors = [];
    page.on('pageerror', (err) => errors.push(String(err)));
    await page.goto(`${BASE}/baam.html`);
    if (javaScriptEnabled) {
      await page.addStyleTag({ content: 'html{scroll-behavior:auto !important}' });
      await page.evaluate(`(${installHelpers.toString()})()`);
      await page.waitForSelector('#ext-featured .ext-card');
      await page
        .waitForFunction(
          () => [...document.querySelectorAll('#extensions-section .ext-card')].every((c) => c.querySelector(':scope > .item-credit')),
          null,
          { timeout: 3000 },
        )
        .catch(() => {});
    }
    return { page, errors, close: () => ctx.close() };
  }

  async function selectShelf(page, shelf) {
    await page.click(`#tab-${shelf}`);
    await page.waitForTimeout(150);
  }

  /**
   * Select a shelf and open what it folds away (the integrations list, or every
   * skill group), so every card on it is laid out. Selecting a shelf folds its
   * skill groups again, so this is the last thing done to a shelf before it is read.
   */
  async function openShelf(page, shelf) {
    await selectShelf(page, shelf);
    if (shelf === 'extensions') {
      const toggle = page.locator('#ext-adapters-toggle');
      if ((await toggle.count()) && (await toggle.isVisible()) && (await toggle.getAttribute('aria-expanded')) !== 'true') await toggle.click();
    } else if (shelf === 'skills') {
      for (const btn of await page.locator('#skills-section .show-all-btn').all()) {
        if ((await btn.isVisible()) && (await btn.getAttribute('aria-expanded')) !== 'true') await btn.click();
      }
    }
    await page.waitForTimeout(150);
  }

  const reports = (page, selector) =>
    page.evaluate(([sel, cur]) => [...document.querySelectorAll(sel)].map((c) => window.__bt.cardReport(c, cur)), [selector, CURATED_BY]);

  /** Step 1's assertions for one shelf's cards, as a list of problems. */
  function cardProblems(list, where) {
    const out = [];
    for (const r of list) {
      const name = `${where} ${r.kind} card ${r.id}`;
      const e = entry(r.kind, r.id);
      if (!e) {
        out.push(`${name} has no entry in credits.json`);
        continue;
      }
      if (r.direct !== 1 || r.all !== 1) {
        out.push(`${name} has ${r.direct} credit lines as children (${r.all} in all); it needs exactly one`);
        continue;
      }
      if (r.tag !== 'p') out.push(`${name}: the credit line is a <${r.tag}>, not a <p>`);
      if (r.text !== e.line) out.push(`${name} reads "${r.text}", credits.json says "${e.line}"`);
      if (!r.afterDesc) out.push(`${name}: the credit line is not the next element after the description`);
      if (!r.cardShown) out.push(`${name}: the card itself does not show with its shelf and group opened`);
      else if (!r.visible) out.push(`${name}: the credit line is not visible`);
      if (!r.belowDesc) out.push(`${name}: the credit line is not below the description`);
      if (!r.aboveTags) out.push(`${name}: the credit line is not above the tag row`);
      if (!r.inCard) out.push(`${name}: the credit line is wider than its card`);
      if (r.overflows) out.push(`${name}: the credit line overflows its own box`);
      if (r.curator) out.push(`${name} carries the curator sentence; it belongs to the shelf, not the card`);
    }
    return out;
  }

  const noSideScroll = (page) => page.evaluate(() => document.documentElement.scrollWidth <= document.documentElement.clientWidth + 1);

  /* ── 1, 3 and 5: the rendered view, in light and dark ─────────────────── */

  for (const scheme of ['light', 'dark']) {
    for (const width of [1440, 390]) {
      test(`every rendered card carries its credit line at ${width}px, ${scheme} scheme`, async () => {
        const { page, errors, close } = await open({ width, scheme });

        await openShelf(page, 'extensions');
        assert.equal(await page.locator('#extensions-grid .ext-card').count(), 0, 'the registry did not render the extension shelf');
        const ext = await reports(page, '#ext-featured .ext-card, #ext-primary .ext-card, #ext-adapters .ext-card');
        assert.deepEqual(
          ext.map((r) => r.id).sort(),
          active('extensions').sort(),
          'the rendered extension shelf is not the set of active extensions in credits.json',
        );
        const extProblems = cardProblems(ext, 'rendered');
        assert.ok(await noSideScroll(page), `the extensions shelf scrolls sideways at ${width}px`);

        await openShelf(page, 'skills');
        const featured = await reports(page, '#skills-featured .skill-card');
        const grids = await reports(page, '#skills-section .skill-grid .skill-card');
        assert.equal(featured.length, 3, 'the featured strip has three skill cards');
        assert.deepEqual(grids.map((r) => r.id).sort(), active('skills').sort(), 'the skill groups are not the set of active skills in credits.json');
        const skillProblems = cardProblems([...featured, ...grids], 'rendered');
        assert.ok(await noSideScroll(page), `the skills shelf scrolls sideways at ${width}px`);

        assert.deepEqual(
          [...extProblems.slice(0, 12), ...skillProblems.slice(0, 12)],
          [],
          `${extProblems.length} extension and ${skillProblems.length} skill card line problems (the first 12 of each)`,
        );

        const low = [...ext, ...featured, ...grids].filter((r) => r.contrast !== null && r.contrast < MIN_CONTRAST);
        assert.deepEqual(
          low.map((r) => `${r.kind} ${r.id}: ${r.contrast}:1`).slice(0, 10),
          [],
          `credit lines under ${MIN_CONTRAST}:1 in the ${scheme} scheme`,
        );
        assert.deepEqual(errors, [], 'the page threw');
        await close();
      });

      test(`the curator line sits once on each shelf and in the footer at ${width}px, ${scheme} scheme`, async () => {
        const { page, errors, close } = await open({ width, scheme });
        assert.equal(await page.locator('.shelf-curator').count(), SHELVES.length, 'one curator line per shelf, no more');
        assert.equal(await page.locator('.foot-curator').count(), 1, 'one curator line in the footer');
        for (const shelf of SHELVES) {
          await selectShelf(page, shelf);
          const seen = await page.evaluate(
            ([id, cur]) => {
              const s = document.getElementById(id);
              const lines = [...s.querySelectorAll('.shelf-curator')];
              const p = lines[0];
              if (p) p.scrollIntoView({ block: 'center' });
              return {
                count: lines.length,
                tag: p ? p.tagName.toLowerCase() : null,
                last: !!p && s.lastElementChild === p,
                text: p ? p.textContent : null,
                visible: window.__bt.visible(p),
                contrast: p ? window.__bt.contrast(p) : null,
                inCard: !!p && !!p.closest('.ext-card, .skill-card, .wf-card'),
                cur,
              };
            },
            [`${shelf}-section`, CURATED_BY],
          );
          assert.equal(seen.count, 1, `#${shelf}-section has ${seen.count} curator lines`);
          assert.equal(seen.tag, 'p', `#${shelf}-section's curator line is a <p>`);
          assert.ok(seen.last, `#${shelf}-section's curator line is not the shelf's last element`);
          assert.equal(seen.text, CURATED_BY, `#${shelf}-section's curator line`);
          assert.ok(seen.visible, `#${shelf}-section's curator line does not show while the shelf is selected`);
          assert.ok(!seen.inCard, `#${shelf}-section's curator line sits inside a card`);
          assert.ok(seen.contrast >= MIN_CONTRAST, `#${shelf}-section's curator line paints at ${seen.contrast}:1 in the ${scheme} scheme`);
        }
        const foot = await page.evaluate(() => {
          const s = document.querySelector('.site-footer .foot-curator');
          if (s) s.scrollIntoView({ block: 'center' });
          return {
            tag: s ? s.tagName.toLowerCase() : null,
            inBrand: !!s && !!s.closest('.foot-brand'),
            text: s ? s.textContent : null,
            visible: window.__bt.visible(s),
            contrast: s ? window.__bt.contrast(s) : null,
          };
        });
        assert.equal(foot.tag, 'span', "the footer's curator line is a <span>");
        assert.ok(foot.inBrand, "the footer's curator line is not in the brand column");
        assert.equal(foot.text, CURATED_BY, "the footer's curator line");
        assert.ok(foot.visible, "the footer's curator line does not show");
        assert.ok(foot.contrast >= MIN_CONTRAST, `the footer's curator line paints at ${foot.contrast}:1 in the ${scheme} scheme`);

        // Owner ruling Q1: the curator sentence is on the shelf and in the footer,
        // never on a card (an opened panel carries it; a closed row does not).
        const onCards = await page.evaluate(
          (cur) =>
            [...document.querySelectorAll('.ext-card, .skill-card, .wf-card')]
              .filter((c) => window.__bt.rowText(c).includes(cur))
              .map((c) => window.__bt.idOf(c) || c.querySelector('h3')?.textContent),
          CURATED_BY,
        );
        assert.deepEqual(onCards, [], 'cards that carry the curator sentence');
        assert.deepEqual(errors, [], 'the page threw');
        await close();
      });
    }
  }

  /* ── 1b. The skill card's own parts around the line ──────────────────── */

  // The credit line took a grid row of its own, and the version chip that
  // baam.js adds to a skill's name block had no placed row on a phone: it fell
  // into the first free one, now under the credit line, where it read as part of
  // the credit (or, on an open row, under the whole panel). And a licence chip
  // longer than a phone's card (replication-package-audit's five licences) ran
  // out of the card, since a chip did not wrap.
  for (const width of [1440, 640, 390]) {
    test(`a skill's version stays with its name, and its chips stay in its card, at ${width}px`, async () => {
      const { page, errors, close } = await open({ width });
      await openShelf(page, 'skills');
      const versions = await page.evaluate(() =>
        [...document.querySelectorAll('#skills-section .skill-card')]
          .filter((c) => window.__bt.visible(c))
          .map((c) => {
            const ver = c.querySelector('.skill-ver');
            const desc = c.querySelector(':scope > .skill-desc');
            const type = c.querySelector('.skill-type');
            const v = ver && ver.getBoundingClientRect();
            return {
              id: window.__bt.idOf(c),
              has: !!ver && !!c.dataset.version,
              visible: window.__bt.visible(ver),
              belowType: !!v && v.top >= type.getBoundingClientRect().top - 0.5,
              aboveDesc: !!v && v.bottom <= desc.getBoundingClientRect().top + 0.5,
            };
          }),
      );
      assert.ok(versions.length > 100, `only ${versions.length} skill cards laid out`);
      const misplaced = versions.filter((r) => !r.has || !r.visible || !r.belowType || !r.aboveDesc).map((r) => r.id);
      assert.deepEqual(misplaced.slice(0, 10), [], `${misplaced.length} skill cards whose version is missing or not between the type line and the description`);

      const row = await cardFor(page, 'skills', 'replication-package-audit');
      const before = await row.evaluate((c) => c.querySelector('.skill-ver').getBoundingClientRect().top - c.getBoundingClientRect().top);
      await row.locator(':scope > .row-toggle').click();
      await page.waitForTimeout(150);
      const opened = await row.evaluate((c) => {
        const k = c.getBoundingClientRect();
        const chips = [...c.querySelectorAll('.skill-tags .tag')].map((t) => {
          const r = t.getBoundingClientRect();
          return { text: t.textContent, inCard: r.left >= k.left - 0.5 && r.right <= k.right + 0.5, overflows: t.scrollWidth > t.clientWidth + 1 };
        });
        return { chips, ver: c.querySelector('.skill-ver').getBoundingClientRect().top - k.top };
      });
      assert.ok(opened.chips.some((c) => / AND .* AND /.test(c.text)), 'the open row shows its long licence chip');
      assert.deepEqual(opened.chips.filter((c) => !c.inCard || c.overflows).map((c) => c.text), [], 'chips wider than their card');
      assert.ok(Math.abs(opened.ver - before) <= 0.5, `opening the row moved the version from ${before} to ${opened.ver}`);
      assert.ok(await noSideScroll(page), `the skills shelf scrolls sideways at ${width}px`);
      assert.deepEqual(errors, [], 'the page threw');
      await close();
    });
  }

  /* ── 2. The no-JS view ───────────────────────────────────────────────── */

  for (const width of [1440, 390]) {
    test(`the no-JS view carries every card's line at ${width}px`, async () => {
      const { page, close } = await open({ width, javaScriptEnabled: false });
      const seen = await page.evaluate(() => {
        const slug = (href) => String(href || '').split('?')[0].split('/').pop().replace(/\.(brxt|zip)$/i, '');
        const read = (card, kind) => {
          const desc = card.querySelector(':scope > .ext-desc, :scope > .skill-desc');
          const lines = [...card.querySelectorAll(':scope > .item-credit')];
          const line = lines[0];
          const r = line && line.getBoundingClientRect();
          const id =
            kind === 'skills'
              ? slug(card.querySelector('.skill-dl-btn')?.getAttribute('href'))
              : card.dataset.registryId || slug(card.querySelector('.brxt-chip')?.getAttribute('href'));
          return {
            kind,
            id,
            direct: lines.length,
            all: card.querySelectorAll('.item-credit').length,
            text: line ? line.textContent : null,
            afterDesc: !!line && !!desc && desc.nextElementSibling === line,
            visible: !!r && r.width > 0 && r.height > 0 && getComputedStyle(line).visibility !== 'hidden',
          };
        };
        return {
          ext: [...document.querySelectorAll('#extensions-grid .ext-card')].map((c) => read(c, 'extensions')),
          featured: [...document.querySelectorAll('#skills-featured .skill-card')].map((c) => read(c, 'skills')),
          skills: [...document.querySelectorAll('#skills-section .skill-grid .skill-card')].map((c) => read(c, 'skills')),
          shelfLines: [...document.querySelectorAll('.shelf-curator')].map((p) => p.textContent),
          footLine: document.querySelector('.foot-curator')?.textContent ?? null,
        };
      });
      assert.deepEqual(seen.ext.map((r) => r.id).sort(), active('extensions').sort(), 'the static extension cards are not the active extensions');
      assert.deepEqual(seen.skills.map((r) => r.id).sort(), active('skills').sort(), 'the static skill groups are not the active skills');
      const problems = [];
      for (const r of [...seen.ext, ...seen.featured, ...seen.skills]) {
        const e = entry(r.kind, r.id);
        const name = `static ${r.kind} card ${r.id}`;
        if (!e) problems.push(`${name} has no entry in credits.json`);
        else if (r.direct !== 1 || r.all !== 1) problems.push(`${name} has ${r.direct} credit lines as children (${r.all} in all)`);
        else {
          if (r.text !== e.line) problems.push(`${name} reads "${r.text}", credits.json says "${e.line}"`);
          if (!r.afterDesc) problems.push(`${name}: the credit line is not the next element after the description`);
          if (r.kind === 'extensions' && !r.visible) problems.push(`${name}: the credit line does not show without JavaScript`);
        }
      }
      assert.deepEqual(problems.slice(0, 25), [], `${problems.length} no-JS card line problems`);
      assert.deepEqual(seen.shelfLines, [CURATED_BY, CURATED_BY, CURATED_BY], 'the shelves carry their curator lines without JavaScript');
      assert.equal(seen.footLine, CURATED_BY, 'the footer carries its curator line without JavaScript');
      await close();
    });
  }

  /* ── 4. The provenance row of an opened panel ────────────────────────── */

  /** The card for `id` on its shelf, laid out (its group expanded, its shelf selected). */
  async function cardFor(page, kind, id) {
    await openShelf(page, kind);
    const sel = kind === 'skills' ? '#skills-section .skill-grid .skill-card' : '#extensions-section .ext-card';
    const index = await page.evaluate(([s, want]) => [...document.querySelectorAll(s)].findIndex((c) => window.__bt.idOf(c) === want), [sel, id]);
    assert.ok(index >= 0, `no ${kind} card for ${id} on the page`);
    const row = page.locator(sel).nth(index);
    await row.scrollIntoViewIfNeeded();
    return row;
  }

  /** The credits row of an open card, read into plain data. */
  const panelRow = (row) =>
    row.evaluate((card) => {
      const rows = [...card.querySelectorAll(':scope > .row-detail .detail-row[data-row="credits"]')];
      const r = rows[0];
      if (!r) return { count: 0 };
      const dd = r.querySelector(':scope > dd');
      const items = (ul) =>
        [...ul.querySelectorAll(':scope > li')].map((li) => {
          const a = li.querySelector('a');
          return {
            text: li.textContent,
            link: a ? { cls: a.className, text: a.textContent, href: a.getAttribute('href') } : null,
            visible: window.__bt.visible(li),
          };
        });
      const blocks = [...dd.children].map((el) => {
        const tag = el.tagName.toLowerCase();
        if (tag === 'details') {
          const ul = el.querySelector(':scope > ul');
          return { tag, cls: el.className, summary: el.querySelector(':scope > summary')?.textContent ?? null, open: el.open, items: ul ? items(ul) : [] };
        }
        if (tag === 'ul') return { tag, cls: el.className, items: items(el) };
        return { tag, cls: el.className, text: el.textContent };
      });
      const links = [...r.querySelectorAll('a')].map((a) => ({ text: a.textContent.replace(/\s*↗\s*$/, '').trim(), href: a.getAttribute('href') }));
      const next = r.nextElementSibling;
      return {
        count: rows.length,
        dt: r.querySelector(':scope > dt')?.textContent.trim() ?? null,
        blocks,
        links,
        install: !!card.querySelector(':scope > .row-detail .detail-row[data-row="install"]'),
        beforeInstall: !!next && next.getAttribute('data-row') === 'install',
        imgs: r.querySelectorAll('img').length,
        visible: window.__bt.visible(r),
      };
    });

  /**
   * Everything in an open card's credits row that reaches past the card's edges
   * or overflows its own box, as short descriptions (empty when it all fits).
   * Folded parts are not laid out, so they are read once opened.
   */
  const panelFit = (row) =>
    row.evaluate((card) => {
      const k = card.getBoundingClientRect();
      const r = card.querySelector(':scope > .row-detail .detail-row[data-row="credits"]');
      if (!r) return ['no credits row'];
      const out = [];
      for (const el of [r, ...r.querySelectorAll('*')]) {
        const b = el.getBoundingClientRect();
        if (b.width === 0 && b.height === 0) continue;
        const name = `<${el.tagName.toLowerCase()}${el.className ? ` class="${el.className}"` : ''}> "${el.textContent.trim().slice(0, 40)}"`;
        if (b.left < k.left - 0.5 || b.right > k.right + 0.5) out.push(`${name} spans ${Math.round(b.left)}..${Math.round(b.right)}, the card ${Math.round(k.left)}..${Math.round(k.right)}`);
        if (getComputedStyle(el).display !== 'inline' && el.clientWidth > 0 && el.scrollWidth > el.clientWidth + 1) {
          out.push(`${name} overflows its box by ${el.scrollWidth - el.clientWidth}px`);
        }
      }
      return out;
    });

  const end = (x) => String(x ?? '').replace(/[\s.]+$/, '');
  /** The list a panel leads with (DESIGN 6.5), and the one after it. */
  function panelLists(e) {
    const sources = { items: e.sources, noun: 'sources', name: (s) => s.name };
    const ideas = { items: e.ideas, noun: 'ideas', name: (i) => i.name };
    if (e.origin === 'adapted' || e.origin === 'third-party') return [sources, { ...ideas, heading: 'Ideas from' }];
    if (e.origin === 'ideas-derived') return [ideas, { ...sources, heading: 'Builds on' }];
    return [{ items: [], noun: 'sources' }, { ...sources, heading: 'Builds on' }];
  }

  /** Every way the read panel row differs from `e`, as a list of problems. */
  function panelProblems(seen, e, label) {
    const out = [];
    if (seen.count !== 1) return [`${label} has ${seen.count} provenance rows`];
    if (seen.dt !== PANEL_ROW_LABEL) out.push(`${label}: the row is labelled "${seen.dt}"`);
    if (!seen.visible) out.push(`${label}: the provenance row does not show`);
    if (seen.install && !seen.beforeInstall) out.push(`${label}: the provenance row is not just before the Install row`);
    if (seen.imgs) out.push(`${label}: the provenance row holds ${seen.imgs} img elements`);
    const blocks = [...seen.blocks];
    const take = (pred) => {
      const i = blocks.findIndex(pred);
      return i === -1 ? null : blocks.splice(i, 1)[0];
    };
    const summary = take((b) => b.cls.split(' ').includes('credit-summary'));
    if (e.summary && (!summary || summary.text !== e.summary)) out.push(`${label}: the summary reads "${summary?.text}", credits.json says "${e.summary}"`);
    // The lists, in order: each shows its first three items and folds the rest
    // under "Show all N".
    const lists = seen.blocks.filter((b) => b.tag === 'ul' && b.cls.split(' ').includes('credit-list'));
    let at = 0;
    for (const want of panelLists(e)) {
      if (!want.items.length) continue;
      const ul = lists[at++];
      if (!ul) {
        out.push(`${label}: no list for its ${want.items.length} ${want.noun}`);
        continue;
      }
      const ix = seen.blocks.indexOf(ul);
      if (want.heading && seen.blocks[ix - 1]?.text !== want.heading) out.push(`${label}: the ${want.noun} list is not headed "${want.heading}"`);
      const more = seen.blocks[ix + 1];
      const all = [...ul.items];
      if (ul.items.length !== Math.min(PANEL_SHOWN, want.items.length)) {
        out.push(`${label}: the ${want.noun} list shows ${ul.items.length} items before "Show all", not ${Math.min(PANEL_SHOWN, want.items.length)}`);
      }
      if (want.items.length > PANEL_SHOWN) {
        const title = `Show all ${want.items.length} ${want.noun}`;
        if (!more || more.tag !== 'details' || more.summary !== title) out.push(`${label}: no "${title}" after the first ${PANEL_SHOWN}`);
        else {
          if (more.open) out.push(`${label}: "${title}" is open before it is pressed`);
          all.push(...more.items);
        }
      }
      if (all.length !== want.items.length) out.push(`${label}: lists ${all.length} of its ${want.items.length} ${want.noun}`);
      want.items.forEach((item, k) => {
        const li = all[k];
        if (!li) return;
        const name = want.name(item) ?? '';
        if (!li.text.startsWith(name)) out.push(`${label}: ${want.noun} ${k + 1} reads "${li.text.slice(0, 60)}", not starting "${name}"`);
        if (item.url) {
          if (!li.link || li.link.href !== item.url || li.link.text !== name) out.push(`${label}: ${want.noun} ${k + 1} does not link its name to ${item.url}`);
        } else if (li.link) out.push(`${label}: ${want.noun} ${k + 1} links "${li.link.text}" to ${li.link.href} with no URL in credits.json`);
      });
    }
    for (const [title, items] of [
      ['Data and services', e.data_and_services],
      ['Earlier versions', e.earlier],
    ]) {
      const d = seen.blocks.find((b) => b.tag === 'details' && b.summary === `${title} (${items.length})`);
      if (!items.length) {
        if (seen.blocks.some((b) => b.tag === 'details' && String(b.summary).startsWith(title))) out.push(`${label}: shows "${title}" with nothing in it`);
        continue;
      }
      if (!d) out.push(`${label}: no collapsed "${title} (${items.length})"`);
      else {
        if (d.open) out.push(`${label}: "${title}" is open before it is pressed`);
        if (d.items.length !== items.length) out.push(`${label}: "${title}" lists ${d.items.length} of ${items.length}`);
      }
    }
    if (e.written_for_biorouter.length) {
      const w = seen.blocks.find((b) => String(b.cls).split(' ').includes('credit-written'));
      const want = `Written for Biorouter: ${e.written_for_biorouter.join(', ')}.`;
      if (!w || w.text !== want) out.push(`${label}: no "${want}"`);
    }
    const people = seen.blocks.find((b) => String(b.cls).split(' ').includes('credit-people'));
    if (e.people.length) {
      const want = `People: ${e.people.map((p) => p.name + (p.affiliation ? `, ${p.affiliation}` : '') + (p.role ? ` (${p.role})` : '')).join('; ')}.`;
      if (!people || people.text !== want) out.push(`${label}: the people line reads "${people?.text}", not "${want}"`);
    } else if (people) out.push(`${label}: a people line with no people in credits.json`);
    const curator = seen.blocks.find((b) => String(b.cls).split(' ').includes('credit-curator'));
    if (!curator || curator.text !== CURATED_BY) out.push(`${label}: the curator line reads "${curator?.text}"`);
    const licence = seen.links.filter((l) => l.text === 'Licence files');
    if (e.licence_files && (licence.length !== 1 || licence[0].href !== e.licence_files)) {
      out.push(`${label}: Licence files links ${JSON.stringify(licence.map((l) => l.href))}, credits.json says ${e.licence_files}`);
    }
    const record = seen.links.filter((l) => l.text === 'Full record');
    if (e.record ? record.length !== 1 || record[0].href !== e.record : record.length !== 0) {
      out.push(`${label}: Full record links ${JSON.stringify(record.map((l) => l.href))}, credits.json says ${e.record}`);
    }
    for (const l of seen.links) if (!/^https?:\/\//i.test(l.href || '')) out.push(`${label}: links "${l.text}" to ${l.href}, not an http(s) URL`);
    return out;
  }

  for (const width of [1440, 390])
  for (const [kind, id, what] of PANEL_ITEMS) {
    test(`the ${id} row (${what}) opens on its provenance and credits at ${width}px`, async () => {
      const e = entry(kind, id);
      assert.ok(e, `credits.json has no ${kind} entry ${id}`);
      if (id === 'idcagent') assert.match(e.line, /^Bridges to /, 'idcagent is the bridge case; its line should begin "Bridges to"');
      if (id === 'replication-package-audit') assert.equal(e.sources.length, 10, 'replication-package-audit is the ten-source case');
      const { page, errors, close } = await open({ width });
      const row = await cardFor(page, kind, id);
      const closed = await controlTops(row);
      await row.locator(':scope > .row-toggle').click();
      await page.waitForTimeout(350);
      const opened = await controlTops(row);
      assert.ok(opened.height > closed.height + 20, `${id} grew when opened`);
      // A control beside the name never moves. One under the description (an
      // extension's GitHub link and download, below 860px) moves by exactly what
      // the opened description grew, so the credits row pushes nothing.
      assert.ok(closed.descTop !== null, `${id} has a description`);
      const beside = Object.keys(closed.tops).filter((n) => width >= 880 || closed.tops[n] < closed.descTop);
      assert.ok(beside.includes('row-toggle'), `${id}'s chevron sits beside its name at ${width}px`);
      if (kind === 'skills') assert.ok(beside.includes('skill-dl-btn'), `${id}'s .zip sits beside its name at ${width}px`);
      const grew = opened.descHeight - closed.descHeight;
      for (const [name, top] of Object.entries(closed.tops)) {
        const moved = opened.tops[name] - top;
        if (beside.includes(name)) assert.ok(Math.abs(moved) < 0.5, `${id}: ${name} moved ${top} -> ${opened.tops[name]} when the row opened`);
        else assert.ok(Math.abs(moved - grew) < 0.5, `${id}: ${name} moved ${moved}px when the row opened, its description grew ${grew}px`);
      }
      const seen = await panelRow(row);
      assert.deepEqual(panelProblems(seen, e, `${id} at ${width}px`), []);
      assert.deepEqual(await panelFit(row), [], `${id}'s credits row does not fit its card at ${width}px`);

      // Every "Show all N" opens and shows each of its items.
      const shows = row.locator(':scope > .row-detail .detail-row[data-row="credits"] details.credit-more > summary');
      for (const summary of await shows.all()) {
        if (!/^Show all \d+ /.test(await summary.textContent())) continue;
        await summary.click();
        const hidden = await summary.evaluate((s) => [...s.parentElement.querySelectorAll('li')].filter((li) => !window.__bt.visible(li)).length);
        assert.equal(hidden, 0, `${id}: "${await summary.textContent()}" opened with items still hidden`);
      }
      // Opened whole, every folded list included, the row still fits and the page
      // still has no sideways scroll.
      await row.evaluate((c) => c.querySelectorAll('.detail-row[data-row="credits"] details').forEach((d) => { d.open = true; }));
      await page.waitForTimeout(150);
      assert.deepEqual(await panelFit(row), [], `${id}'s credits row, opened whole, does not fit its card at ${width}px`);
      assert.ok(await noSideScroll(page), `the page scrolls sideways with ${id}'s credits row open at ${width}px`);
      if (id === 'replication-package-audit') {
        // The first three, then the seven "Show all 10 sources" folds away, all shown now.
        const listed = await row.evaluate((card) => {
          const more = [...card.querySelectorAll('.detail-row[data-row="credits"] dd > details.credit-more')].find(
            (d) => d.querySelector(':scope > summary')?.textContent === 'Show all 10 sources',
          );
          if (!more) return [];
          const first = more.previousElementSibling;
          return [...(first ? first.querySelectorAll(':scope > li') : []), ...more.querySelectorAll(':scope > ul > li')]
            .filter((li) => window.__bt.visible(li))
            .map((li) => li.textContent);
        });
        assert.equal(listed.length, 10, `replication-package-audit shows ${listed.length} sources after "Show all 10 sources"`);
        e.sources.forEach((s, k) => assert.ok(listed[k]?.startsWith(s.name), `source ${k + 1} is "${listed[k]}", not ${s.name}`));
      }
      assert.deepEqual(errors, [], 'the page threw');
      await close();
    });
  }

  /* ── 6. Credit values are text ───────────────────────────────────────── */

  test('credit values from credits.json reach the page as text, never as markup or a script link', async () => {
    // DESIGN 7.6's three values (a card line, a source's text, a data entry's
    // terms) and a javascript: source URL, plus every other kind of value the
    // panel writes: its summary, a linked name and a person, so a writer that
    // takes any one of them through innerHTML is caught.
    const poisoned = structuredClone(CREDITS);
    poisoned.extensions.cdwagent.line = PAYLOAD;
    const bio = poisoned.extensions.biomcpagent;
    assert.ok(bio.sources.length && bio.data_and_services.length > 1 && bio.people.length, 'biomcpagent has a source, two data entries and a person to poison');
    assert.match(bio.data_and_services[1].url || '', /^https?:\/\//, "biomcpagent's second data entry has a web URL, so its name is a link");
    bio.sources[0].what = PAYLOAD;
    bio.sources[0].url = BAD_URL;
    bio.data_and_services[0].terms = PAYLOAD;
    bio.data_and_services[1].name = PAYLOAD;
    bio.summary = PAYLOAD;
    bio.people[0].name = PAYLOAD;
    const PLANTED = 5;
    const { page, errors, close } = await open({
      width: 1440,
      credits: (route) => route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(poisoned) }),
    });
    const cdw = await cardFor(page, 'extensions', 'cdwagent');
    const card = await cdw.evaluate((c) => ({
      line: c.querySelector(':scope > .item-credit')?.textContent ?? null,
      imgs: c.querySelectorAll('img').length,
    }));
    assert.equal(card.line, PAYLOAD, "cdwagent's card does not show the served line as literal text");
    assert.equal(card.imgs, 0, "cdwagent's card grew an img element from its line");

    const row = await cardFor(page, 'extensions', 'biomcpagent');
    await row.locator(':scope > .row-toggle').click();
    await page.waitForTimeout(350);
    await row.evaluate((c) => c.querySelectorAll('.detail-row[data-row="credits"] details').forEach((d) => { d.open = true; }));
    await page.waitForTimeout(300);
    const panel = await row.evaluate((c, args) => {
      const r = c.querySelector('.detail-row[data-row="credits"]');
      if (!r) return null;
      const lis = [...r.querySelectorAll('li')];
      const src = lis.find((li) => li.textContent.startsWith(args.name));
      return {
        planted: r.textContent.split(args.payload).length - 1,
        linkedName: [...r.querySelectorAll('a')].some((a) => a.textContent === args.payload && /^https?:\/\//i.test(a.getAttribute('href') || '')),
        imgs: c.querySelectorAll('img').length,
        sourceLinks: src ? src.querySelectorAll('a').length : null,
      };
    }, { name: bio.sources[0].name, payload: PAYLOAD });
    assert.ok(panel, 'biomcpagent opened with no provenance row');
    assert.equal(panel.planted, PLANTED, `the ${PLANTED} served values do not all show as literal text in the panel`);
    assert.ok(panel.linkedName, 'the linked data name does not show as literal text inside its link');
    assert.equal(panel.imgs, 0, "biomcpagent's row grew an img element from a credit value");
    assert.equal(panel.sourceLinks, 0, 'a source whose URL is javascript: is still a link');
    assert.equal(await page.locator('a[href^="javascript:" i]').count(), 0, 'the page carries a javascript: link');
    await page.waitForTimeout(200);
    assert.equal(await page.evaluate(() => window.__pwned), undefined, 'a credit value ran script');
    assert.deepEqual(errors, [], 'the page threw');
    await close();
  });

  /* ── 7. The credit line is not searched ──────────────────────────────── */

  const shownCount = (page, selector) =>
    page.$$eval(selector, (els) => els.filter((e) => e.style.display !== 'none' && e.offsetParent !== null).length);

  test('the search and the tag facets do not read the credit line', async () => {
    const { page, errors, close } = await open({ width: 1440 });
    // Guard: the name searched for must be on the page, in a credit line only, or
    // "no card matched" proves nothing.
    const carriers = await page.$$eval('.item-credit', (els) => els.filter((e) => /Aperivue/.test(e.textContent)).length);
    assert.ok(carriers > 0, 'no credit line names Aperivue, so this search tests nothing');
    for (const [shelf, sel] of [
      ['extensions', '#extensions-section .ext-card'],
      ['skills', '#skills-section .skill-card'],
      ['workflows', '#workflows-section .wf-card'],
    ]) {
      await selectShelf(page, shelf);
      await page.fill('#baam-search', 'Aperivue');
      await page.waitForTimeout(50);
      assert.equal(await shownCount(page, sel), 0, `a search for Aperivue (a credit, not a capability) shows ${shelf} cards`);
      await page.fill('#baam-search', '');
    }

    await selectShelf(page, 'extensions');
    const chips = page.locator('#ext-chips .fchip[data-facet="tag"]');
    const n = await chips.count();
    assert.ok(n > 0, 'the extension shelf has no tag facets to compare');
    const CARDS = '#extensions-section .ext-card';
    const counts = async () => {
      const out = [];
      for (let i = 0; i < n; i++) {
        await chips.nth(i).click();
        out.push([await chips.nth(i).textContent(), await shownCount(page, CARDS)]);
        await chips.nth(i).click();
      }
      return out;
    };
    const total = await page.locator(CARDS).count();
    const before = await counts();

    // A facet word planted in a credit line must not bring its card in. Without
    // this the comparison below passes whatever the facet reads, because no tag
    // word appears in today's lines.
    const narrow = before.findIndex(([, c]) => c > 0 && c < total);
    assert.ok(narrow !== -1, 'no tag facet narrows the shelf, so nothing can be planted');
    const match = await chips.nth(narrow).getAttribute('data-match');
    await chips.nth(narrow).click();
    const planted = await page.evaluate((word) => {
      const card = [...document.querySelectorAll('#extensions-section .ext-card')].find((c) => c.style.display === 'none' && c.querySelector(':scope > .item-credit'));
      if (!card) return null;
      card.querySelector(':scope > .item-credit').textContent += ` ${word}`;
      return window.__bt.idOf(card);
    }, match);
    await chips.nth(narrow).click();
    await chips.nth(narrow).click();
    assert.ok(planted, 'no hidden card has a credit line to plant a word in');
    const plantedShown = await page.evaluate((id) => {
      const card = [...document.querySelectorAll('#extensions-section .ext-card')].find((c) => window.__bt.idOf(c) === id);
      return card.style.display !== 'none';
    }, planted);
    assert.equal(plantedShown, false, `the "${match}" facet shows ${planted} for a word planted in its credit line`);
    await chips.nth(narrow).click();

    await page.evaluate(() => document.querySelectorAll('#extensions-section .item-credit').forEach((p) => p.remove()));
    const after = await counts();
    assert.deepEqual(after, before, 'the tag facets show different cards once the credit lines are gone');
    assert.deepEqual(errors, [], 'the page threw');
    await close();
  });

  /* ── 8. credits.json answers 404 ─────────────────────────────────────── */

  for (const width of [1440, 390])
  test(`with credits.json missing, the rendered cards keep their lines and a panel names line and curator at ${width}px`, async () => {
    const { page, errors, close } = await open({ width, credits: (route) => route.fulfill({ status: 404, body: 'not found' }) });
    await openShelf(page, 'extensions');
    assert.equal(await page.locator('#extensions-grid .ext-card').count(), 0, 'the registry did not render the extension shelf');
    const ext = await reports(page, '#ext-featured .ext-card, #ext-primary .ext-card, #ext-adapters .ext-card');
    assert.deepEqual(ext.map((r) => r.id).sort(), active('extensions').sort());
    const problems = ext
      .filter((r) => r.direct !== 1 || r.text !== entry('extensions', r.id)?.line || !r.afterDesc || !r.visible)
      .map((r) => `${r.id}: ${r.direct} lines, "${r.text}"`);
    assert.deepEqual(problems, [], 'rendered cards that lost their harvested line');

    for (const [kind, id] of [
      ['extensions', 'cdwagent'],
      ['skills', 'scientific-research'],
    ]) {
      const row = await cardFor(page, kind, id);
      await row.locator(':scope > .row-toggle').click();
      await page.waitForTimeout(350);
      const seen = await row.evaluate((c) => {
        const r = c.querySelector(':scope > .row-detail .detail-row[data-row="credits"]');
        if (!r) return null;
        return {
          dt: r.querySelector('dt')?.textContent.trim(),
          paragraphs: [...r.querySelectorAll('dd p')].map((p) => p.textContent),
          curator: r.querySelector('.credit-curator')?.textContent ?? null,
          links: [...r.querySelectorAll('a')].map((a) => a.getAttribute('href')),
        };
      });
      assert.ok(seen, `${id} opened with no provenance row while credits.json is missing`);
      assert.equal(seen.dt, PANEL_ROW_LABEL);
      assert.ok(seen.paragraphs.includes(entry(kind, id).line), `${id}'s row does not name its line: ${JSON.stringify(seen.paragraphs)}`);
      assert.equal(seen.curator, CURATED_BY, `${id}'s row does not carry the curator line`);
      for (const href of seen.links) assert.match(href || '', /^https?:\/\//i, `${id}'s row links ${href}`);
      assert.deepEqual(await panelFit(row), [], `${id}'s fallback credits row does not fit its card at ${width}px`);
      await row.locator(':scope > .row-toggle').click();
    }
    assert.deepEqual(errors, [], 'the page threw');
    await close();
  });
}
