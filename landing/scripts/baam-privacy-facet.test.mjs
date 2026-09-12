// Browser-level regression suite for the BAAM shelf's privacy badges + facet.
//
//     node --test landing/scripts/baam-privacy-facet.test.mjs
//
// Why a real browser and not jsdom. Half of what this file asserts are LAYOUT
// facts, and jsdom has no layout — every getBoundingClientRect() it returns is
// zeros, so a clipped tag row and a perfect one are indistinguishable there.
// `.ext-tags` is `max-height: 22px; overflow: hidden`, which means an extra chip
// does not push the row taller, it silently deletes the last tag from view. That
// failure is invisible in a diff and invisible in a screenshot of any card that
// happened to have two tags. Only a browser that lays the row out can see it.
//
// Two mutants this suite is built to kill, both of which an earlier version of
// it let through:
//
//   * `filterExtensions` matching the privacy facet on a SUBSTRING of the card's
//     prose instead of `dataset.privacy`. The card text now contains the badge
//     word "Private", so the naive "does the Private chip show exactly cdwagent
//     and ucsfomopagent" assertion passes either way. Planting the word on a
//     public card is what separates them.
//   * `trimTagRows` hiding more than overflowed. "No chip is clipped" is
//     trivially satisfiable by hiding every chip, so the trim is asserted as a
//     property — surviving chips are a prefix, and re-showing the first dropped
//     chip must overflow the row — rather than as a bound on how many are left.
//
// Playwright is not a landing/ dependency — it is resolved out of
// ui/desktop/node_modules, and the browser is whatever this machine already has
// (headless shell, the bundled chromium, or a system Chrome). If none launches
// the tests FAIL rather than skip: a silent skip is how a layout gate becomes a
// decoration.
//
// Where this runs. `just check-shelf` locally; the `shelf` job in
// .github/workflows/frontend.yml on every PR and push; and — the one that
// matters — a step in the `check` job of .github/workflows/deploy-landing.yml,
// which the Pages deploy depends on. `landing/` is published as-is, with no
// build step between this repo and biorouter.ucsf.edu, so a gate the deploy does
// not consult is a gate the site can regress straight past. It is NOT in
// `just check-everything` / `just check-registry`, which run on a bare checkout
// with no npm install and no browser.

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { createReadStream, existsSync, statSync } from 'node:fs';
import { dirname, extname, join, normalize, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const SCRIPTS = dirname(fileURLToPath(import.meta.url));
const LANDING = resolve(SCRIPTS, '..');
const REPO = resolve(LANDING, '..');
const PLAYWRIGHT = join(REPO, 'ui/desktop/node_modules/playwright/index.mjs');

const TYPES = {
  '.html': 'text/html; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.mjs': 'text/javascript; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.json': 'application/json; charset=utf-8',
  '.svg': 'image/svg+xml',
  '.png': 'image/png',
};

/** Serve landing/ over loopback so `fetch('registry.json')` resolves. */
function serveLanding() {
  const server = createServer((req, res) => {
    const path = decodeURIComponent((req.url || '/').split('?')[0]);
    const file = join(LANDING, normalize(path).replace(/^(\.\.[/\\])+/, ''));
    if (!file.startsWith(LANDING) || !existsSync(file) || !statSync(file).isFile()) {
      res.writeHead(404).end('not found');
      return;
    }
    res.writeHead(200, { 'content-type': TYPES[extname(file)] || 'application/octet-stream' });
    createReadStream(file).pipe(res);
  });
  return new Promise((ok) => server.listen(0, '127.0.0.1', () => ok(server)));
}

/**
 * Whatever chromium this machine can actually start. Playwright's default
 * headless mode wants a `chrome-headless-shell` download that a repo which only
 * uses Playwright for Electron E2E may never have fetched.
 */
async function launchChromium(chromium) {
  const attempts = [{}, { channel: 'chromium' }, { channel: 'chrome' }];
  const failures = [];
  for (const opts of attempts) {
    try {
      return await chromium.launch(opts);
    } catch (err) {
      failures.push(`${JSON.stringify(opts)}: ${String(err).split('\n')[0]}`);
    }
  }
  throw new Error(
    'no chromium could be launched, so the layout assertions in this file cannot run:\n  ' +
      failures.join('\n  ') +
      '\nInstall one with: cd ui/desktop && npx playwright install chromium'
  );
}

if (!existsSync(PLAYWRIGHT)) {
  test('playwright is available', () => {
    assert.fail(
      `${PLAYWRIGHT} is missing — run \`cd ui/desktop && npm install\`. ` +
        'These are layout assertions; jsdom cannot stand in for them.'
    );
  });
} else {
  const { chromium } = await import(PLAYWRIGHT);
  const server = await serveLanding();
  const BASE = `http://127.0.0.1:${server.address().port}`;
  const browser = await launchChromium(chromium);

  test.after(async () => {
    await browser.close();
    server.close();
  });

  const PRIVATE = '#ext-chips .fchip[data-facet="privacy"][data-match="private"]';
  const PUBLIC = '#ext-chips .fchip[data-facet="privacy"][data-match="public"]';

  /** A page with the registry-rendered shelf already in the DOM. */
  async function shelfPage(width = 1280) {
    const page = await browser.newPage({ viewport: { width, height: 900 } });
    await page.goto(`${BASE}/baam.html`);
    // renderExtensions() empties the static grid once the registry lands.
    await page.waitForSelector('#ext-featured .ext-card');
    return page;
  }

  /** Every card the shelf is currently showing, named the way a human would. */
  const shownCards = (page) =>
    page.$$eval('#extensions-section .ext-card:visible', (els) =>
      els.map((e) => e.dataset.extensionName || e.querySelector('h3').textContent).sort()
    );

  test('the privacy facet filters the shelf down to the two private extensions', async () => {
    const page = await shelfPage();
    await page.click(PRIVATE);
    assert.deepEqual(await shownCards(page), ['cdwagent', 'ucsfomopagent']);
    await page.close();
  });

  test('the privacy facet matches the card’s tier, not the word "private" in its prose', async () => {
    // The discrimination this facet exists for, and the one assertion the
    // previous version of this file could not make. `filterExtensions` builds a
    // `hay` string out of the card's textContent — which now CONTAINS the badge
    // word "Private" — so restoring the old substring branch still returns
    // exactly these two extensions and every other test here still passes.
    // Plant the word on a public card and the two branches finally disagree.
    const page = await shelfPage();
    const planted = await page.evaluate(() => {
      const card = document.querySelector('#ext-primary .ext-card[data-privacy="public"]');
      card.querySelector('.ext-desc').textContent += ' Runs against a private clone of the index.';
      card.dataset.tags = `${card.dataset.tags || ''} private`;
      return card.querySelector('h3').textContent;
    });
    await page.click(PRIVATE);
    const shown = await shownCards(page);
    assert.equal(
      shown.includes(planted),
      false,
      `"${planted}" is public and answered the Private chip on a substring of its prose`
    );
    assert.deepEqual(shown, ['cdwagent', 'ucsfomopagent']);
    await page.close();
  });

  test('the Public facet is the complement, and both chips together are the whole shelf', async () => {
    const page = await shelfPage();
    const all = await page.evaluate(() =>
      [...document.querySelectorAll('#extensions-section .ext-card')].map(
        (e) => e.dataset.extensionName || e.querySelector('h3').textContent
      )
    );

    await page.click(PUBLIC);
    const pub = await shownCards(page);
    assert.equal(pub.includes('cdwagent'), false, 'a private extension answered the Public chip');
    assert.equal(pub.includes('ucsfomopagent'), false);
    assert.equal(pub.length, all.length - 2, 'Public must be exactly everything that is not private');

    // Both selected is a union inside one facet, not an impossible AND.
    await page.click(PRIVATE);
    assert.deepEqual(await shownCards(page), all.slice().sort());
    await page.close();
  });

  test('a card with three real tags still shows all three', async () => {
    const page = await shelfPage();
    const chips = await page.$$eval(
      '.ext-card[data-extension-name="cdwagent"] .ext-tags > span',
      (els) =>
        els
          .filter((e) => e.getBoundingClientRect().width > 0)
          .map((e) => e.textContent)
    );
    // EXACT, not "at most 3". `chips.length <= 3` is satisfied by a row that
    // shows nothing but the badge, which is precisely the over-trimming this
    // file has to be able to fail on.
    //
    // "UCSF data" is the institution badge and it REPLACES the plain "UCSF"
    // subject tag rather than joining it: the two say the same word for the same
    // reason, and side by side they read as a rendering fault. The badge is the
    // one derived from a declaration, so the subject tag is the one that gives
    // way — which is also what keeps this row at the same three chips it had
    // before the badge existed.
    assert.deepEqual(chips, ['Private', 'UCSF data', 'MCP']);

    // The clip test: every laid-out chip must sit inside the 22px row. A chip
    // that wrapped is not "a bit cramped", it is invisible — and the row's
    // `overflow: hidden` means no scrollbar, no ellipsis, nothing to notice.
    const clipped = await page.$$eval('#extensions-section .ext-card .ext-tags > span', (els) =>
      els
        .filter((e) => e.getBoundingClientRect().height > 0)
        .filter((e) => e.getBoundingClientRect().bottom > e.parentElement.getBoundingClientRect().bottom + 0.5)
        .map((e) => `${e.closest('.ext-card').querySelector('h3').textContent}: ${e.textContent}`)
    );
    assert.deepEqual(clipped, [], 'these chips fell outside the clipped .ext-tags row');
    await page.close();
  });

  test('the tag row is trimmed by exactly what does not fit, and never further', async () => {
    // `trimTagRows()` keeps the row inside its 22px by HIDING chips, so "nothing
    // is clipped" is trivially satisfiable by hiding everything. Hiding chips
    // that would have fitted is the same silent deletion the clip test exists to
    // catch, one mechanism over — so assert the property directly: the surviving
    // chips are a prefix, and re-showing the first dropped chip must overflow the
    // row. `slice(0, 0)`, "hide everything after the badge", and an off-by-one in
    // the trim loop all fail here and nowhere else.
    for (const width of [1280, 390]) {
      const page = await shelfPage(width);
      const bad = await page.$$eval('#extensions-section .ext-card:visible .ext-tags', (rows) =>
        rows
          .filter((row) => row.clientHeight > 0)
          .map((row) => {
            const name = row.closest('.ext-card').querySelector('h3').textContent;
            const chips = [...row.querySelectorAll('span')];
            const isHidden = (c) => c.style.display === 'none';
            const firstHidden = chips.findIndex(isHidden);
            if (firstHidden !== -1 && chips.slice(firstHidden).some((c) => !isHidden(c))) {
              return `${name}: a hidden chip sits before a visible one — the row is not a prefix`;
            }
            if (firstHidden === -1) return null;
            if (firstHidden === 0) return `${name}: the privacy badge itself was trimmed away`;
            const chip = chips[firstHidden];
            chip.style.display = '';
            const overflows = row.scrollHeight > row.clientHeight;
            chip.style.display = 'none';
            return overflows ? null : `${name}: "${chip.textContent}" was dropped but fits`;
          })
          .filter(Boolean)
      );
      assert.deepEqual(bad, [], `over-trimmed tag rows at ${width}px`);
      await page.close();
    }
  });

  test('the no-JS view labels every card, at both geometries', async () => {
    // The static cards are the fallback view AND the registry generator's input.
    // Without JS nothing re-renders and nothing trims, so whatever the markup
    // says is what a visitor reads — and an unbadged card there reads as "not yet
    // reviewed" rather than "public", which is exactly the ambiguity the badge
    // was added to remove. Badging only the two private cards leaves the other
    // 35 saying nothing.
    const ctx = await browser.newContext({ javaScriptEnabled: false });
    for (const width of [1280, 390]) {
      const page = await ctx.newPage();
      await page.setViewportSize({ width, height: 900 });
      await page.goto(`${BASE}/baam.html`);

      assert.equal(
        await page.locator('.ext-card[data-privacy="private"] .tag.private').count(),
        2,
        'the two private cards must declare their tier in markup'
      );

      const wrong = await page.$$eval('#extensions-section .ext-card', (els) =>
        els
          .map((e) => {
            const name = e.querySelector('h3').textContent;
            const badges = [...e.querySelectorAll('.ext-tags > span[data-privacy-badge]')];
            if (badges.length !== 1) return `${name}: ${badges.length} privacy badges, expected 1`;
            const [badge] = badges;
            const want = e.dataset.privacy === 'private' ? 'Private' : 'Public';
            if (badge.textContent !== want) {
              return `${name}: the badge says "${badge.textContent}" but the card declares ${want}`;
            }
            // The badge is authored FIRST precisely so the row's
            // `overflow: hidden` eats subject tags rather than the tier. Nothing
            // trims this view, so first-or-clipped is the whole guarantee.
            const row = badge.parentElement;
            if (badge !== row.firstElementChild) return `${name}: the badge is not the first chip`;
            const r = badge.getBoundingClientRect();
            if (r.width === 0 || r.bottom > row.getBoundingClientRect().bottom + 0.5) {
              return `${name}: the badge is clipped out of the tag row`;
            }
            return null;
          })
          .filter(Boolean)
      );
      assert.deepEqual(wrong, [], `unlabelled or mislabelled no-JS cards at ${width}px`);

      // The institution badge, same view and the same guarantee. It is authored
      // SECOND — right behind the tier it qualifies — so `overflow: hidden` eats
      // subject tags before it, and it must be laid out inside the row at both
      // geometries. A badge that wrapped is not cramped, it is deleted, and the
      // reader is left thinking the connector carries no institutional
      // constraint at all.
      const badgeWrong = await page.$$eval('#extensions-section .ext-card', (els) =>
        els
          .map((e) => {
            const name = e.querySelector('h3').textContent;
            const declared = (e.dataset.affiliation || '').split(/\s+/).filter(Boolean);
            const chips = [...e.querySelectorAll('.ext-tags > span[data-affiliation-badge]')];
            const ids = chips.map((c) => c.dataset.affiliationBadge);
            // Absent means unconstrained. An empty badge on an unaffiliated card
            // would read as a constraint that does not exist, which is worse
            // than saying nothing.
            if (ids.join(' ') !== declared.join(' ')) {
              return `${name}: badges [${ids}] but declares data-affiliation="${declared.join(' ')}"`;
            }
            const row = e.querySelector('.ext-tags');
            for (const chip of chips) {
              if (chip.previousElementSibling !== row.firstElementChild &&
                  chip.previousElementSibling.dataset.affiliationBadge === undefined) {
                return `${name}: the institution badge does not follow the privacy badge`;
              }
              const r = chip.getBoundingClientRect();
              if (r.width === 0 || r.bottom > row.getBoundingClientRect().bottom + 0.5) {
                return `${name}: the institution badge is clipped out of the tag row`;
              }
            }
            return null;
          })
          .filter(Boolean)
      );
      assert.deepEqual(badgeWrong, [], `institution badges wrong in the no-JS view at ${width}px`);
      await page.close();
    }
    await ctx.close();
  });

  test('the rendered shelf states affiliation the same way the authored cards do', async () => {
    // The bug this exists for: the authored cards carried `data-affiliation` and
    // `extCardHtml` did not, so the two views of the same card disagreed the
    // moment the registry landed. A badge driven off that attribute would have
    // painted once, over the static markup, and vanished on the re-render — the
    // hardest possible failure to see, because the page is correct until the
    // fetch resolves.
    const page = await shelfPage();
    const bad = await page.evaluate(() => {
      const out = [];
      const cards = [...document.querySelectorAll('#extensions-section .ext-card')];
      if (cards.length === 0) out.push('the shelf rendered no cards at all');
      for (const card of cards) {
        const name = card.querySelector('h3').textContent;
        const declared = (card.dataset.affiliation || '').split(/\s+/).filter(Boolean);
        const chips = [...card.querySelectorAll('.ext-tags > span[data-affiliation-badge]')];
        // Absent means unconstrained: a card with no affiliation must render NO
        // institution badge, not an empty one. `data-affiliation=""` is also the
        // spelling the generator refuses outright, so an attribute present and
        // empty is itself the failure.
        if (card.getAttribute('data-affiliation') === '') {
          out.push(`${name}: data-affiliation is present but empty`);
        }
        if (chips.length !== declared.length) {
          out.push(`${name}: ${chips.length} institution badges for ${declared.length} declared`);
          continue;
        }
        chips.forEach((chip, i) => {
          if (chip.dataset.affiliationBadge !== declared[i]) {
            out.push(`${name}: badge ${i} is "${chip.dataset.affiliationBadge}", declared "${declared[i]}"`);
          }
          // The DISPLAY NAME, never the id. "ucsf data" is a slug that escaped
          // onto the public marketplace.
          if (/^[a-z0-9-]+ data$/.test(chip.textContent) && chip.textContent === `${declared[i]} data`) {
            out.push(`${name}: badge ${i} renders the raw id "${chip.textContent}"`);
          }
        });
      }
      return out;
    });
    assert.deepEqual(bad, []);

    // …and the two UCSF connectors really do carry it, so the assertions above
    // are not vacuously true of a shelf that renders no affiliation anywhere.
    const cdw = page.locator('#ext-featured .ext-card[data-extension-name="cdwagent"]');
    assert.equal(await cdw.getAttribute('data-affiliation'), 'ucsf');
    assert.equal(
      await cdw.locator('.ext-tags > span[data-affiliation-badge="ucsf"]').textContent(),
      'UCSF data',
      "the badge must render registry.json's institutions[id], not the id"
    );
    assert.equal(
      await page.locator('#extensions-section .ext-card[data-affiliation]').count(),
      2,
      'exactly the two private UCSF connectors declare an affiliation today'
    );
    // The complement, stated directly: 35 unconstrained cards, no badge on any
    // of them. An institution badge with a fallback label would satisfy every
    // assertion above and fail this one.
    assert.equal(
      await page.locator(
        '#extensions-section .ext-card:not([data-affiliation]) .ext-tags > span[data-affiliation-badge]'
      ).count(),
      0,
      'an unaffiliated card must render no institution badge — absent means unconstrained'
    );
    await page.close();
  });

  /** The shelf after `registry.json` has been rewritten in flight. */
  async function shelfWithRegistry(mutate) {
    const page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
    await page.route('**/registry.json', async (route) => {
      const body = await (await route.fetch()).json();
      mutate(body);
      await route.fulfill({ json: body });
    });
    await page.goto(`${BASE}/baam.html`);
    // Either outcome has cards in the DOM: rendering empties #extensions-grid,
    // refusing to render leaves the authored cards exactly where they were.
    await page.waitForSelector('#extensions-section .ext-card');
    return page;
  }

  const renderedFromRegistry = (page) =>
    page.$$eval('#extensions-grid .ext-card', (els) => els.length === 0);

  test('a registry entry with an unreadable tier keeps the static fallback', async () => {
    // `privacy === 'private' ? 'private' : 'public'` reads every value it does
    // not recognise as Public, so one unparseable field paints a reassuring
    // label nobody computed — on a shelf whose whole point is which extensions
    // touch private data. The generator refuses to PUBLISH such a value; the
    // renderer has to refuse to DISPLAY it, because the file it renders is
    // fetched at runtime and the page it is fetched into cannot re-validate the
    // source. The correct answer already exists: the authored cards underneath.
    const page = await shelfWithRegistry((r) => {
      r.extensions[0].privacy = 'privte';
    });
    assert.equal(
      await renderedFromRegistry(page),
      false,
      'a registry with an unreadable tier must not reach the DOM'
    );
    // …and the fallback is intact, not a blank shelf.
    assert.equal(
      await page.locator('.ext-card[data-privacy="private"] .tag.private').count(),
      2
    );
    await page.close();
  });

  // The three affiliations `build-registry.mjs` hard-fails on. It cannot protect
  // this page: registry.json is fetched at runtime from a server the generator
  // never ran against. Rendering must not paper over any of them — a chip built
  // from a broken declaration states a constraint nobody approved, and quietly
  // dropping the chip states the opposite. The authored cards underneath are the
  // answer in both cases.
  const UNRENDERABLE_AFFILIATIONS = [
    ['an institution the map does not name', (e) => { e.affiliation = ['atlantis']; }],
    ['an affiliation on a public extension', (e, r) => {
      const pub = r.extensions.find((x) => x.privacy !== 'private');
      pub.affiliation = ['ucsf'];
    }],
    ['an empty affiliation list', (e) => { e.affiliation = []; }],
  ];
  for (const [what, mutate] of UNRENDERABLE_AFFILIATIONS) {
    test(`a registry with ${what} keeps the static fallback`, async () => {
      const page = await shelfWithRegistry((r) => {
        mutate(r.extensions.find((e) => e.extension_name === 'cdwagent'), r);
      });
      assert.equal(
        await renderedFromRegistry(page),
        false,
        `a registry with ${what} must not reach the DOM`
      );
      assert.equal(
        await page.locator('.ext-card[data-privacy="private"] .tag.private').count(),
        2,
        'the fallback is the authored cards, not a blank shelf'
      );
      await page.close();
    });
  }

  test('an institution map that is missing entirely keeps the static fallback', async () => {
    // The label comes out of `registry.institutions`, so a payload that declares
    // affiliations without the map has no display name to render — and the
    // failure mode is `undefined data` painted on the marketplace, which looks
    // like a bug in the card rather than a bad catalog.
    const page = await shelfWithRegistry((r) => { delete r.institutions; });
    assert.equal(await renderedFromRegistry(page), false);
    await page.close();
  });

  test('both badges are a visible pill in dark as well as light', async () => {
    // `.tag.private` is the navy ramp: `background: rgba(5,32,73,0.07)`, a 7%
    // tint that reads as a soft chip on a white card. `landing/theme.js` sets
    // `.dark` pre-paint and rebinds `--ucsf` to a light steel blue, so the TEXT
    // survives — but the background is a literal, not a token, and 7% navy over
    // a #1e1811 card composites to within four counts of the card itself. The
    // Private badge lost its pill in dark while Public kept one, which is
    // exactly backwards: private is the tier that has to stand out.
    for (const scheme of ['light', 'dark']) {
      const ctx = await browser.newContext({ colorScheme: scheme, viewport: { width: 1280, height: 900 } });
      const page = await ctx.newPage();
      await page.goto(`${BASE}/baam.html`);
      await page.waitForSelector('#ext-featured .ext-card');
      const seen = await page.evaluate(() => {
        // Rasterise rather than parse. A computed background can come back as
        // `rgba(5,32,73,0.07)` or as `color(srgb 0.56 0.7 0.87 / 0.18)`, and a
        // regex that assumes 0-255 reads the second one as almost no colour at
        // all — which looks exactly like the bug being tested for. Painting the
        // chip over the card and reading the pixel back asks the browser to do
        // both the parsing and the alpha compositing.
        const cv = document.createElement('canvas');
        cv.width = cv.height = 1;
        const ctx2d = cv.getContext('2d', { willReadFrequently: true });
        const paint = (...layers) => {
          ctx2d.clearRect(0, 0, 1, 1);
          for (const css of layers) {
            ctx2d.fillStyle = css;
            ctx2d.fillRect(0, 0, 1, 1);
          }
          return [...ctx2d.getImageData(0, 0, 1, 1).data].slice(0, 3);
        };
        const card = getComputedStyle(document.querySelector('.ext-card')).backgroundColor;
        const base = paint(card);
        // The largest per-channel gap between the chip and the card under it.
        const chip = (sel) => {
          const on = paint(card, getComputedStyle(document.querySelector(sel)).backgroundColor);
          return Math.max(...on.map((c, i) => Math.abs(c - base[i])));
        };
        return {
          dark: document.documentElement.classList.contains('dark'),
          private: chip('.tag.private'),
          public: chip('.tag.public'),
          // The institution badge joined the same navy ramp, so it inherits the
          // same dark-mode failure: a 7% navy literal over the #1e1811 card is
          // no pill at all. It is only covered because it was added to the dark
          // override list beside .tag.private, and nothing but this asserts that.
          affiliation: chip('.tag.affiliation'),
        };
      });
      assert.equal(seen.dark, scheme === 'dark', 'theme.js did not follow the colour scheme');
      assert.ok(seen.private >= 8, `the Private badge is ${seen.private.toFixed(1)}/255 from the card in ${scheme} — no visible pill`);
      assert.ok(seen.public >= 8, `the Public badge is ${seen.public.toFixed(1)}/255 from the card in ${scheme} — no visible pill`);
      assert.ok(seen.affiliation >= 8, `the institution badge is ${seen.affiliation.toFixed(1)}/255 from the card in ${scheme} — no visible pill`);
      await ctx.close();
    }
  });


  /**
   * Searching the shelf must not search the LICENCE. Every card in this catalog
   * is Apache-2.0 and the catalog publishes that licence three more times per
   * card — as `data-license`, as a chip in the tag row (part of the card's own
   * textContent), and on a skill as the `apache` keyword in `data-tags` — so a
   * haystack built from all of them answered "apache" with the whole shelf.
   *
   * The same overlap in the app's own matchers is worse, because those split a
   * query into words and fall a plural back to its singular: measured in the
   * Browse-extensions modal on 2026-09-12, `PACS` returned 31 of 37 extensions
   * through `pac` inside `apache`, none of them about PACS.
   *
   * Driven through the real input, not by calling `filterExtensions` — the
   * `oninput` attribute, `runFilter`'s trim/lowercase and the shelf's own
   * visibility rules are all part of what a person experiences here.
   */
  async function search(page, query) {
    await page.fill('#baam-search', query);
    await page.waitForTimeout(0);
    return shownCards(page);
  }

  test('a licence is not something a card is searched by', async () => {
    const page = await shelfPage();

    // Guard: every assertion below is vacuous if the licence stops reaching the
    // haystack, which is what is being closed. On a RENDERED extension card it
    // arrives two ways — `data-license`, and the `Apache-2.0` token inside the
    // `data-tags` keyword blob. (Not as a chip: `extCardHtml` already drops an
    // `/^apache/i` tag from the tag row, for space rather than for search, and
    // that one hard-coded filter is exactly why this defect looked fixed.)
    const licenceCarriers = await page.evaluate(() =>
      [...document.querySelectorAll('#extensions-section .ext-card')].filter((card) => {
        const licence = (card.dataset.license || '').toLowerCase();
        if (!licence) return false;
        return (card.dataset.tags || '')
          .split(/\s+/)
          .some((token) => token.toLowerCase() === licence);
      }).length
    );
    assert.ok(
      licenceCarriers >= 2,
      `only ${licenceCarriers} cards carry their own licence into the haystack — this test proves nothing`
    );

    const all = await search(page, '');
    for (const query of ['apache', 'Apache-2.0', 'APACHE']) {
      assert.deepEqual(
        await search(page, query),
        [],
        `"${query}" is a licence, not a capability — it matched cards before this fix`
      );
    }

    // And nothing else moved: a real tag, a name, a data source, and browsing.
    assert.deepEqual(await search(page, ''), all);
    assert.ok((await search(page, 'MCP')).length >= 10, 'a real tag must still match');
    assert.deepEqual(await search(page, 'spokeagent'), ['spokeagent']);
    assert.ok((await search(page, 'imaging')).length >= 2, '`imaging` is a capability, not a licence');
    await page.close();
  });

  test('a skill card is not searched by its licence either', async () => {
    // The skills shelf carries the licence a third way — the `apache` keyword in
    // `data-tags`, which is not spelled like the `Apache-2.0` chip, so a rule
    // comparing a label to the licence for EQUALITY leaves this one matching.
    const page = await shelfPage();
    await page.click('.baam-tab[data-shelf="skills"]');
    await page.waitForSelector('#skills-section .skill-card');

    const visibleSkills = () =>
      page.$$eval('#skills-section .skill-card:visible', (els) => els.length);
    const keyworded = await page.evaluate(() =>
      [...document.querySelectorAll('#skills-section .skill-card')].filter((card) =>
        (card.dataset.tags || '').split(/\s+/).includes('apache')
      ).length
    );
    assert.ok(keyworded >= 2, `only ${keyworded} skill cards carry an \`apache\` keyword`);
    // A skill card is authored, not rendered, so it DOES wear the licence chip —
    // the one path that lives inside `textContent` rather than a data attribute.
    const chipped = await page.evaluate(() =>
      [...document.querySelectorAll('#skills-section .skill-card')].filter((card) =>
        [...card.querySelectorAll('.tag')].some(
          (chip) =>
            chip.textContent.trim().toLowerCase() === (card.dataset.license || '').toLowerCase()
        )
      ).length
    );
    assert.ok(chipped >= 2, `only ${chipped} skill cards wear their own licence as a chip`);

    await page.fill('#baam-search', 'apache');
    assert.equal(await visibleSkills(), 0, 'a licence keyword matched skill cards');

    await page.fill('#baam-search', 'ggplot');
    assert.ok((await visibleSkills()) >= 1, '`ggplot` must still match');
    await page.close();
  });

  test('a well-formed registry still renders', async () => {
    // Without this, "refuse to render" is satisfiable by never rendering, and
    // every other test in this file would be reading static markup.
    const page = await shelfWithRegistry(() => {});
    assert.equal(await renderedFromRegistry(page), true);
    await page.close();
  });
}
