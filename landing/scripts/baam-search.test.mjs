// The BAAM shelves answer a phrase, not a substring.
//
//     node --test landing/scripts/baam-search.test.mjs
//
// Two halves, and both are needed.
//
// The first half drives the REAL page in a browser. That is where the defect
// lived and where a regression would land: `filterExtensions`, `filterSkills`
// and `filterWorkflows` each asked `hay.indexOf(q) !== -1`, so a visitor who
// typed the way people type — `R scripting ggplot visualization` — got the empty
// shelf and a "No skills match your search." line. A unit test of the matcher
// cannot see that, because the matcher was never the thing on the page.
//
// The second half tests `landing/marketplace-search.js` directly, as a module.
// It is the port of `crates/biorouter/src/marketplace/search.rs`, and the rules
// that keep a union of terms from returning the whole catalog (short terms are
// whole-word only; filler is dropped; license is not a searched field) are
// cheapest to pin one rule at a time, without a browser.
//
// Playwright is not a landing/ dependency — it is resolved out of
// ui/desktop/node_modules, exactly as baam-privacy-facet.test.mjs does it, and
// the browser is whatever this machine already has. If none launches the browser
// half FAILS rather than skips; the module half still runs.

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { createReadStream, existsSync, statSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, extname, join, normalize, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const SCRIPTS = dirname(fileURLToPath(import.meta.url));
const LANDING = resolve(SCRIPTS, '..');
const REPO = resolve(LANDING, '..');
const PLAYWRIGHT = join(REPO, 'ui/desktop/node_modules/playwright/index.mjs');

const require = createRequire(import.meta.url);
const Search = require(join(LANDING, 'marketplace-search.js'));

/* ── The matcher, on its own ──────────────────────────────────────────── */

const ENTRIES = [
  {
    id: 'r-scripting',
    name: 'R Scripting',
    description: 'Tidyverse conventions for R code.',
    tags: ['R'],
  },
  {
    id: 'ggplot-visualization',
    name: 'ggplot2 Visualization',
    description: 'Publication-quality figures and plots.',
    tags: ['ggplot2'],
  },
  {
    id: 'complex-plots',
    name: 'Complex Plots',
    description: 'Draws annotated heat maps with the ComplexHeatmap package.',
    tags: ['ComplexHeatmap'],
  },
  {
    id: 'prose-only',
    name: 'Prose Only',
    description: 'Mentions scripting in passing. Licensed Apache-2.0.',
    tags: [],
    license: 'Apache-2.0',
  },
];

/** The searched fields of an entry — note that `license` is not among them. */
const entryFields = (entry) => [
  [entry.id, Search.Weight.Name],
  [entry.name, Search.Weight.Name],
  [entry.description, Search.Weight.Prose],
  ...entry.tags.map((tag) => [tag, Search.Weight.Label]),
];

const ids = (query) =>
  Search.rank(query, Search.SKILL_NOISE, ENTRIES, entryFields).hits.map((hit) => hit.entry.id);

test('a phrase matches any of its terms, not the whole string', () => {
  // The measured query, and the reason this module exists. Under the substring
  // matcher this was zero.
  assert.deepEqual(ids('R scripting ggplot visualization'), [
    'r-scripting',
    'ggplot-visualization',
    'prose-only',
  ]);
});

test('a query is split at punctuation as well as whitespace, and lowercased', () => {
  assert.deepEqual(Search.terms('R-scripting, GGPLOT2!', Search.SKILL_NOISE), [
    'r',
    'scripting',
    'ggplot2',
  ]);
});

test('a term under three characters matches whole words only', () => {
  // `r` has to find the R language. As a substring it is inside "figures",
  // "draws" and most of the catalog — so only one entry matches the TERM.
  const found = Search.rank('r', Search.SKILL_NOISE, ENTRIES, entryFields);
  assert.deepEqual(
    found.hits.filter((hit) => hit.matchedTerms.length > 0).map((hit) => hit.entry.id),
    ['r-scripting']
  );
  // The others are here only because a one-letter query occurs verbatim in
  // their prose, which is the canonical matcher's own first rank — so the R
  // language still comes first.
  assert.equal(ids('r')[0], 'r-scripting');
});

test('filler words are dropped, but a query made only of filler still searches', () => {
  assert.deepEqual(Search.terms('a skill about R', Search.SKILL_NOISE), ['r']);
  assert.deepEqual(Search.terms('how to use it', Search.SKILL_NOISE), [
    'how',
    'to',
    'use',
    'it',
  ]);
});

test('a plural falls back to its singular', () => {
  assert.ok(ids('visualizations').includes('ggplot-visualization'));
  assert.ok(ids('heatmaps').includes('complex-plots'));
});

test('a term nothing matches returns nothing, never the whole catalog', () => {
  assert.deepEqual(ids('kubernetes'), []);
});

test('the singular fallback is a fallback, not a prefix search', () => {
  // `PACS` is the measured query that must not return the catalog. It ends in
  // `s`, so the plural rule tries `pac` — which really is inside "package" and
  // "apache". That is the canonical rule, and the point is that it finds two
  // entries rather than all four.
  assert.deepEqual(ids('PACS'), ['complex-plots', 'prose-only']);
});

test('the verbatim phrase ranks first', () => {
  assert.equal(ids('annotated heat maps')[0], 'complex-plots');
});

test('license is not a searched field', () => {
  // `prose-only` says "Apache-2.0" in its description, so it is found; nothing
  // is found by a `license` key, because `entryFields` never offers one.
  const found = Search.rank('Apache-2.0', Search.SKILL_NOISE, ENTRIES, (entry) => [
    [entry.id, Search.Weight.Name],
    [entry.name, Search.Weight.Name],
    [entry.tags.join(' '), Search.Weight.Label],
  ]);
  assert.deepEqual(found.hits, []);
});

test('an empty query is the browse case: every entry, in the order given', () => {
  assert.deepEqual(ids('   '), ENTRIES.map((entry) => entry.id));
});

/* ── The real page ────────────────────────────────────────────────────── */

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

/** Whatever chromium this machine can actually start. */
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
    'no chromium could be launched, so the page-level assertions cannot run:\n  ' +
      failures.join('\n  ') +
      '\nInstall one with: cd ui/desktop && npx playwright install chromium'
  );
}

if (!existsSync(PLAYWRIGHT)) {
  test('playwright is available', () => {
    assert.fail(
      `${PLAYWRIGHT} is missing — run \`cd ui/desktop && npm install\`. ` +
        'The page-level half of this file cannot run without it.'
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

  /** A page with the registry-rendered shelf already in the DOM. */
  async function shelfPage(shelf) {
    const page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
    await page.goto(`${BASE}/baam.html`);
    await page.waitForSelector('#ext-featured .ext-card');
    if (shelf) await page.click(`.baam-tab[data-shelf="${shelf}"]`);
    return page;
  }

  /** Type `query` into the one search box and read back what survives. */
  async function search(page, query, selector) {
    await page.fill('#baam-search', query);
    return page.$$eval(selector, (els) =>
      els.map((e) => e.dataset.extensionName || e.querySelector('h3').textContent.trim())
    );
  }

  const SKILLS = '#skills-section .skill-card:visible';
  const EXTS = '#extensions-section .ext-card:visible';

  test('a natural-language skills query finds the skills it names', async () => {
    const page = await shelfPage('skills');
    const shown = await search(page, 'R scripting ggplot visualization', SKILLS);
    assert.ok(shown.length > 0, 'the phrase returned an empty shelf');
    assert.ok(shown.includes('R Scripting'), `R Scripting missing from: ${shown.join(', ')}`);
    assert.ok(
      shown.includes('ggplot2 Visualization'),
      `ggplot2 Visualization missing from: ${shown.join(', ')}`
    );
    await page.close();
  });

  test('a natural-language extensions query finds the extension it names', async () => {
    const page = await shelfPage();
    const shown = await search(page, 'SPOKE knowledge graph', EXTS);
    assert.ok(shown.length > 0, 'the phrase returned an empty shelf');
    assert.ok(
      shown.some((name) => /spoke/i.test(name)),
      `SPOKEAgent missing from: ${shown.join(', ')}`
    );
    await page.close();
  });

  test('a term the catalog does not use does not list every skill', async () => {
    const page = await shelfPage('skills');
    const total = await page.$$eval('#skills-section .skill-card', (els) => els.length);
    const shown = await search(page, 'PACS', SKILLS);
    assert.ok(
      shown.length < total,
      `"PACS" showed all ${total} skills — the union is matching on nothing`
    );
    await page.close();
  });

  test('a one-word query still works', async () => {
    // The shape the substring matcher got right. Nothing it returned is lost.
    const page = await shelfPage('skills');
    const shown = await search(page, 'ggplot', SKILLS);
    assert.ok(shown.includes('ggplot2 Visualization'), shown.join(', '));
    await page.close();
  });

  test('the license chip is not searchable', async () => {
    // "R Scripting" carries `data-license="Apache-2.0"` and says apache nowhere
    // else — not in its heading, its prose, its type line or its two tags. The
    // old matcher folded `data-license` into the haystack and returned it.
    const page = await shelfPage('skills');
    const shown = await search(page, 'apache', SKILLS);
    assert.ok(
      !shown.includes('R Scripting'),
      'a card matched on its license alone, which is not a searched field'
    );
    await page.close();
  });

  test('an emptied box restores the whole shelf', async () => {
    const page = await shelfPage('skills');
    const total = await page.$$eval('#skills-section .skill-card', (els) => els.length);
    await search(page, 'ggplot', SKILLS);
    await page.fill('#baam-search', '');
    const restored = await page.$$eval('#skills-section .skill-card', (els) =>
      els.filter((e) => e.style.display !== 'none').length
    );
    assert.equal(restored, total);
    await page.close();
  });
}
