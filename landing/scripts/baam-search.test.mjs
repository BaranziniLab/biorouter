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
// It is the port of `crates/biorouter/src/catalog_search.rs` (`marketplace/search.rs`
// until PR #266 moved it), and the rules
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
import { readFile } from 'node:fs/promises';
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

// The verbatim bonus asks whether the phrase is WRITTEN IN the field, not
// whether it occurs in it — `written_in` in catalog_search.rs, which PR #266
// introduced after this port was first written. These are that module's own six
// assertions, run against the port, so the two cannot drift apart unnoticed.
//
// The shelves call `matching`, which collapses rank to a membership set, so this
// rule changes nothing a visitor sees today. It is pinned because the port claims
// to be rule for rule, and a claim nothing checks is the one that goes stale.
test('the verbatim bonus respects word boundaries, as the canonical matcher does', () => {
  const { writtenIn } = Search;
  assert.equal(writtenIn('r scripting', 'r scripting'), true, 'the whole phrase');
  assert.equal(writtenIn('tidy code for r.', 'r'), true, 'the second `r`, at a boundary');
  assert.equal(writtenIn('snippets for scripting', 'r scripting'), false, 'not present');
  assert.equal(writtenIn('tidyverse', 'dy'), false, 'buried inside a longer word');
  assert.equal(
    writtenIn('ba a a', 'a a'),
    true,
    'a refused occurrence can overlap an accepted one, so every position is tried'
  );
  assert.equal(
    writtenIn('c++ code', '++'),
    true,
    'a non-alphanumeric edge imposes no boundary on that side'
  );
});

// An unanchored match has to be worth something. `substantial_infix` in
// catalog_search.rs and `substantialInfix` in the desktop port assert the same
// words; this is the third copy of that rule and so the third copy of the test.
test('a short term matches inside a word only when it is half of it', () => {
  const { substantialInfix } = Search;
  // The three floods measured on this page, at the word each came through.
  assert.equal(substantialInfix('lab', 'baranzinilab'), false, '3 of 12');
  assert.equal(substantialInfix('gen', 'cdwagent'), false, '3 of 8');
  assert.equal(substantialInfix('age', 'language'), false, '3 of 8');
  // A short term inside a SHORT word is the search, not a morpheme — the hits a
  // flat four-character floor would have cost.
  assert.equal(substantialInfix('rna', 'scrna'), true, '3 of 5');
  assert.equal(substantialInfix('sem', 'rsem'), true, '3 of 4');
  // At four characters a term is unanchored anywhere, however long the word,
  // which is what keeps a compound biomedical vocabulary findable.
  assert.equal(substantialInfix('omics', 'transcriptomics'), true);
  assert.equal(substantialInfix('flow', 'workflows'), true);
  // The case the infix rule was written for sits exactly on the short arm's
  // boundary, so it would pass on either arm.
  assert.equal(substantialInfix('heatmap', 'complexheatmap'), true, '7 of 14');
  // And only the UNANCHORED match is refused: the start of a word still counts
  // at three characters, and the whole word always counts.
  const strengthOf = (term, word) =>
    Search.rank(term, [], [{ id: 'x', name: word, description: '', tags: [] }], entryFields).hits
      .length;
  assert.equal(strengthOf('gen', 'genomics'), 1, 'a prefix still matches');
  assert.equal(strengthOf('lab', 'lab'), 1, 'a whole word still matches');
  assert.equal(strengthOf('gen', 'cdwagent'), 0, 'an infix of a long word does not');
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

  /* ── The three matchers, differentially ─────────────────────────────────
     The rule lives in three places — `crates/biorouter/src/catalog_search.rs`,
     `ui/desktop/src/components/baam/search.ts` and `marketplace-search.js` — and
     what each one SEARCHES lives in a fourth: the field list its caller hands
     over. That is where the three had drifted. `catalog_search.rs` and the
     desktop port searched a skill's `category`; this page never did, so a
     visitor and a model reading the same catalog got different answers to
     `core` — 59 of 129 there against 2 of 132 here.

     A unit test of any one matcher cannot see that, so the contract is restated
     HERE, independently, as `expectedFields` below, and every card the real page
     shows is compared against it over the whole catalog vocabulary. A fourth
     copy is the point: if any implementation drifts from the contract, this
     fails, and it fails whichever of the four moved. */

  /** The canonical searched fields, per `MarketplaceCatalog::search_extensions`. */
  const extensionFields = (entry) => [
    [entry.id, Search.Weight.Name],
    [entry.extension_name, Search.Weight.Name],
    [entry.name, Search.Weight.Name],
    [entry.organization, Search.Weight.Label],
    [entry.description, Search.Weight.Prose],
    ...(entry.tags || [])
      .filter((tag) => !namesOnlyTheLicense(tag, entry.license))
      .map((tag) => [tag, Search.Weight.Label]),
  ];

  /**
   * The canonical searched fields, per `MarketplaceCatalog::search_skills`.
   * Neither `category` nor `type` is here: each is a curation value the page
   * answers with a facet chip, and each named most of the shelf.
   */
  const skillFields = (entry) => [
    [entry.id, Search.Weight.Name],
    [entry.name, Search.Weight.Name],
    [entry.description, Search.Weight.Prose],
    ...(entry.tags || [])
      .filter((tag) => !namesOnlyTheLicense(tag, entry.license))
      .map((tag) => [tag, Search.Weight.Label]),
    ...(entry.keywords || [])
      .filter((kw) => !namesOnlyTheLicense(kw, entry.license))
      .map((kw) => [kw, Search.Weight.Label]),
  ];

  /** A label that says nothing its entry's own licence does not. */
  function namesOnlyTheLicense(label, license) {
    const labelWords = Search.words(label);
    if (labelWords.length === 0) return false;
    const licenseWords = Search.words(license || '');
    return labelWords.every((word) => licenseWords.includes(word));
  }

  /**
   * What a visitor might type: every distinct word the catalog itself uses, plus
   * the queries that measured the four defects this file guards. Derived from the
   * registry rather than listed, so a new entry widens the comparison.
   */
  function corpus(registry) {
    const words = new Set([
      'lab', 'gen', 'age', 'core', 'cor', 'ore', 'biomedical', 'developer',
      'invocable', 'auto', 'user', 'applied', 'apache', 'Apache-2.0', 'PACS',
      'rna', 'omics', 'flow', 'heatmap', 'UCSF', 'BaranziniLab', 'SPOKEAgent',
      'single cell', 'variant calling', 'R scripting ggplot visualization',
    ]);
    const add = (value) => {
      if (typeof value === 'string') for (const word of Search.words(value)) words.add(word);
      else if (Array.isArray(value)) value.forEach(add);
    };
    for (const entry of registry.extensions) {
      [entry.id, entry.name, entry.organization, entry.tags].forEach(add);
    }
    for (const entry of registry.skills) {
      [entry.id, entry.name, entry.tags, entry.keywords].forEach(add);
    }
    return [...words].sort();
  }

  /**
   * Every query's visible cards, keyed by download URL, read out of the real page
   * in ONE round trip. `oninput="runFilter()"` is the shelf's own entry point, so
   * this drives exactly what typing drives; doing it per query over Playwright
   * would be ~800 round trips.
   */
  async function shelfAnswers(page, cardSelector, queries) {
    return page.evaluate(
      ({ cardSelector, queries }) => {
        const box = document.getElementById('baam-search');
        const cards = [...document.querySelectorAll(cardSelector)];
        const urlOf = (card) => {
          const link = card.querySelector('.skill-dl-btn, .brxt-chip');
          return link ? link.getAttribute('href') : '';
        };
        const out = { '': cards.map(urlOf) };
        for (const query of queries) {
          box.value = query;
          box.dispatchEvent(new Event('input', { bubbles: true }));
          // A filtered shelf un-collapses, so every card it kept is displayed;
          // `.skill-grid.collapsed > .skill-card:nth-child(n+9)` is the same
          // `display: none` a refused card gets, which is why the browse case is
          // read off the DOM above rather than asked for here.
          out[query] = cards
            .filter((card) => getComputedStyle(card).display !== 'none')
            .map(urlOf);
        }
        box.value = '';
        box.dispatchEvent(new Event('input', { bubbles: true }));
        return out;
      },
      { cardSelector, queries }
    );
  }

  /**
   * ⚠ The skills comparison reads the three GRIDS, not the whole shelf.
   * `build-registry.mjs` derives every registry row from `#core-skill-grid`,
   * `#dev-skill-grid` and `#bio-skill-grid`, so those cards stand one-to-one
   * against the rows. The `#skills-featured` strip above them repeats three of
   * those skills as hand-written cards, and the copies have DRIFTED — measured on
   * the live page, the featured `ggplot2 Visualization` reads "Publication-quality
   * ggplot2 figures in R — font sizing, palettes, themes" where its grid twin
   * reads "Applies ggplot2 best-practice style", and carries a `Figures` tag and
   * seven `data-tags` the grid card has none of. That is prose the registry does
   * not describe, so a differential against the registry cannot speak about it: it
   * is a content divergence on the page, not a matcher one.
   */
  for (const [label, shelf, selector, key, fields, noise] of [
    ['extensions', 'extensions', '#extensions-section .ext-card', 'extensions', extensionFields, Search.EXTENSION_NOISE],
    ['skills', 'skills', '#skills-section .skill-grid .skill-card', 'skills', skillFields, Search.SKILL_NOISE],
  ]) {
    test(`the ${label} shelf answers every catalog query the canonical field list does`, async () => {
      const registry = JSON.parse(await readFile(join(LANDING, 'registry.json'), 'utf8'));
      const entries = registry[key];
      const page = await shelfPage(shelf === 'extensions' ? null : shelf);
      const queries = corpus(registry);
      assert.ok(queries.length > 300, `the corpus is only ${queries.length} queries`);

      // A card is identified by the download link the registry row carries, so the
      // comparison is between two sets of ROWS and never depends on DOM order.
      const answers = await shelfAnswers(page, selector, queries);
      const cards = new Set(answers['']);
      assert.ok(!cards.has(''), 'a card was not identified by its download link');
      assert.equal(cards.size, answers[''].length, `${label}: two cards share a download link`);
      assert.equal(cards.size, entries.length, `${label}: cards drawn vs registry rows`);

      const slug = (url) => url.split('/').pop();
      const mismatches = [];
      for (const query of queries) {
        const want = Search.rank(query, noise, entries, fields).hits.map((hit) => hit.entry.download);
        const got = answers[query];
        const extra = got.filter((url) => !want.includes(url)).map(slug);
        const missing = want.filter((url) => !got.includes(url)).map(slug);
        if (extra.length || missing.length) {
          mismatches.push(
            `${query}: page ${got.length}, canonical ${want.length}` +
              (extra.length ? `; page only ${extra.join(',')}` : '') +
              (missing.length ? `; canonical only ${missing.join(',')}` : '')
          );
        }
      }
      assert.deepEqual(
        mismatches,
        [],
        `${mismatches.length} of ${queries.length} ${label} queries disagree with the canonical ` +
          `field list:\n  ${mismatches.slice(0, 25).join('\n  ')}`
      );
      await page.close();
    });
  }

  test('a three-letter query does not return the whole extensions shelf', async () => {
    // Measured on this page against the live 37-entry registry before the fix:
    // `lab` 37 of 37 (through an infix of `BaranziniLab` in the org line), `gen`
    // and `age` 36 each (through an infix of `…Agent` in the heading).
    const page = await shelfPage();
    const total = await page.$$eval('#extensions-section .ext-card', (els) => els.length);
    assert.ok(total >= 30, `measured against 37 cards; now ${total}`);
    for (const [query, was] of [['lab', 37], ['gen', 36], ['age', 36]]) {
      const shown = await search(page, query, EXTS);
      assert.ok(
        shown.length <= 8,
        `"${query}" showed ${shown.length} of ${total} cards (${was} before): ${shown.join(', ')}`
      );
    }
    // What a visitor browses this shelf BY has to survive, and all three reach
    // their cards as whole words.
    const lab = await search(page, 'BaranziniLab', EXTS);
    assert.ok(lab.length >= 20, `the lab by its own name: ${lab.length} of ${total}`);
    const ucsf = await search(page, 'UCSF', EXTS);
    assert.ok(ucsf.length >= 5 && ucsf.length < total, `UCSF: ${ucsf.length} of ${total}`);
    assert.deepEqual(await search(page, 'SPOKEAgent', EXTS), ['spokeagent']);
    await page.close();
  });

  test('the invocation mode is a facet, and the slug beside it is still searched', async () => {
    // `.skill-type` is "User-invocable · /scientific-research" — a MODE, which
    // `initSkills` reads into `card._type` for the two chips to filter on, and a
    // SLUG, which is the skill's registry id. The whole line went in at Name
    // weight, so the mode — an administrative label on 100% of cards — was
    // searched: measured here, `invocable` showed 62 of 132 and `auto` 74, while
    // the app answered 0 and 4. `auto` is a real topical query (autoimmune,
    // automation, autoencoder), so that one cost a search a visitor makes.
    const page = await shelfPage('skills');
    const total = await page.$$eval('#skills-section .skill-card', (els) => els.length);
    for (const [query, was] of [['invocable', 62], ['auto', 74], ['user', 62], ['applied', 70]]) {
      const shown = await search(page, query, SKILLS);
      assert.ok(
        shown.length * 4 < total,
        `"${query}" showed ${shown.length} of ${total} cards (${was} before the mode was dropped)`
      );
    }
    // The slug is the half worth keeping: it is the only place a skill's id is
    // rendered, and two of these skills say their id nowhere else on the card.
    for (const slug of ['ucsf-hpc', 'scientific-machine-learning', 'gpu-compute-optimization']) {
      const shown = await search(page, slug, SKILLS);
      assert.ok(shown.length >= 1, `the slug /${slug} found nothing`);
    }
    await page.close();
  });

  test('a skills category is a filter chip, not a searched word', async () => {
    // This page was already right, and is pinned so it stays the side the other
    // two were brought to: `Core` names 57 of the registry's 129 skills and
    // `Biomedical` 63, so searching the bucket returned half the shelf — which
    // is what `catalog_search.rs` and the desktop modal were doing (59 and 65).
    const page = await shelfPage('skills');
    const total = await page.$$eval('#skills-section .skill-card', (els) => els.length);
    for (const query of ['core', 'biomedical', 'developer']) {
      const shown = await search(page, query, SKILLS);
      assert.ok(
        shown.length * 4 < total,
        `"${query}" showed ${shown.length} of ${total} cards, i.e. its whole bucket`
      );
    }
    // The bucket is still reachable — by its chip, which is where it belongs.
    await page.fill('#baam-search', '');
    await page.click('.fchip[data-facet="category"][data-match="developer"]');
    const chipped = await page.$$eval(SKILLS, (els) =>
      els.filter((e) => getComputedStyle(e).display !== 'none').length
    );
    assert.ok(chipped > 0 && chipped * 4 < total, `the Developer chip showed ${chipped}`);
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
