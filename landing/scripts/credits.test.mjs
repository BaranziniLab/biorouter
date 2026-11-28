// Tests for the credits contract (lib/credits.mjs) and its page writer
// (write-credits.mjs). No browser and no network: the page is read as text.
//
//     node --test landing/scripts/credits.test.mjs
//
// What they pin, and why each matters:
//
//   * credits.json's shape and key order. It is published beside registry.json
//     and copied into the app byte for byte, so a reordered key is a diff in
//     two committed files and a reader that walks keys sees a different record.
//   * pinText, httpUrl and escapeText, the three functions that stand between a
//     record and what a page shows. httpUrl is why a record can never put a
//     javascript: or data: link on the page.
//   * writeCreditsHtml and regeneratePage are idempotent on the real page, so
//     the generator can run on every sync without churning baam.html.
//   * creditPageProblems (rule PV14's page half) fails each way a line can go
//     wrong: changed, missing, doubled, misplaced, a missing shelf or footer
//     line, a stray class, and credit wording written by hand. Each case is a
//     mutant of a page that passes, and the unmutated page is asserted clean
//     first, so a checker that had quietly become `return []` fails here.

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import {
  CARD_CREDIT_CLASS,
  CREDITS_PATHS,
  CREDITS_VERSION,
  CURATED_SHELVES,
  buildCredits,
  cardCreditSites,
  creditPageProblems,
  creditsEntry,
  escapeText,
  handWrittenCredit,
  httpUrl,
  pinText,
  serializeCredits,
  writeCreditsHtml,
} from './lib/credits.mjs';
import { CURATED_BY, MADE_BY_LAB, creditLine } from './lib/catalog-rules.mjs';
import { loadCatalogs, regeneratePage, writeSkillCardFields } from './write-credits.mjs';

const SCRIPTS = dirname(fileURLToPath(import.meta.url));
const LANDING = resolve(SCRIPTS, '..');
const REAL_PAGE = readFileSync(join(LANDING, 'baam.html'), 'utf8');
const CATALOGS = loadCatalogs(join(LANDING, 'catalog'));

// ---------------------------------------------------------------- fixtures

const ENTRY_KEYS = [
  'status', 'origin', 'line', 'summary', 'licence', 'sources', 'ideas', 'data_and_services', 'people',
  'written_for_biorouter', 'earlier', 'moved_to', 'licence_files', 'record',
];
const SOURCE_KEYS = ['name', 'url', 'pin', 'licence', 'credit', 'people', 'use', 'what', 'owner', 'via'];
const IDEA_KEYS = ['name', 'url', 'credit', 'people', 'what', 'cite'];
const DATA_KEYS = ['kind', 'name', 'url', 'terms', 'credit', 'cite', 'how'];
const PERSON_KEYS = ['name', 'affiliation', 'role'];

/** An adapted skill bundle with one of everything a credits entry carries. */
const ADAPTED_BUNDLE = {
  id: 'toy-bundle',
  kind: 'bundle',
  status: 'active',
  version: '1.2.0',
  licence: 'Apache-2.0 AND MIT',
  path: 'toy-bundle',
  notice_file: 'LICENSE-UPSTREAM',
  source: {
    origin: 'adapted',
    upstreams: [
      {
        kind: 'repository', repo: 'someone/toys', url: 'https://github.com/someone/toys', display: 'toys',
        commit: '0123456789abcdef0123456789abcdef01234567', licence: 'MIT', credit: 'Some One', people: ['Some One', ' '],
        use: ['text'], what: 'Two reference files.', via: ['other/origin'],
      },
      { kind: 'package', name: 'evil', url: 'javascript:alert(1)', version: 'v2.0.1', licence: 'MIT', credit: 'Evil Corp', use: ['dependency'], what: 'Runs it.' },
    ],
    ideas_from: [
      { name: 'Idea Now', url: 'https://example.org/now', credit: 'Idea Person', applies_to: 'current', what: 'The layout.' },
      { name: 'Idea Then', url: 'data:text/html,hi', credit: 'Old Person', applies_to: 'earlier', what: 'The first topic list.' },
    ],
    data_and_services: [{ kind: 'service', name: 'PubMed', url: 'https://pubmed.ncbi.nlm.nih.gov/', terms: 'NCBI terms.', credit: 'NLM', how: 'Searched.' }],
  },
  components: {
    'written-here': { name: 'Written Here', status: 'active', origin: 'in-house' },
    'idea-shaped': { status: 'active', origin: 'ideas-derived' },
    'copied': { name: 'Copied', status: 'active', origin: 'adapted' },
    'gone': { name: 'Gone', status: 'removed', origin: 'in-house' },
  },
  contributors: [{ name: 'Wanjun Gu', affiliation: 'Baranzini Lab, UCSF', role: 'curator' }],
  history: [
    { version: '1.0.0', origin: 'in-house', licence: 'Apache-2.0' },
    { version: '1.2.0', origin: 'adapted', licence: 'Apache-2.0 AND MIT' },
  ],
};

const EXTENSION = {
  id: 'toyagent',
  status: 'active',
  version: '0.2.0',
  licence: 'Apache-2.0',
  repository: 'BaranziniLab/ToyAgent',
  release: { tag: 'v0.2.0' },
  source: {
    origin: 'third-party',
    upstreams: [{ kind: 'program', display: 'Toy MCP', repo: 'toy/toy-mcp', version: '1.0.0', licence: 'MIT', credit: 'Toy <Org>', use: ['launched'], what: 'Launched.' }],
  },
  contributors: [],
};

const TOY_CATALOGS = {
  extensions: { format: 2, items: [EXTENSION] },
  skills: { format: 2, source_commit: 'c'.repeat(40), items: [ADAPTED_BUNDLE] },
};

/**
 * The real page with the curator elements placed, as baam.html carries them
 * once the credits-and-page lane placed them by hand: a no-op on that page,
 * and what lets these tests run on a page from before.
 */
function withCurators(html) {
  let out = html;
  const next = { extensions: 'workflows-section', workflows: 'skills-section', skills: 'baam-moved' };
  assert.deepEqual(CURATED_SHELVES, ['extensions-section', 'workflows-section', 'skills-section']);
  for (const shelf of ['extensions', 'workflows', 'skills']) {
    const at = out.indexOf(`id="${shelf}-section"`);
    const end = out.indexOf(`id="${next[shelf]}"`, at);
    if (out.slice(at, end).includes('class="shelf-curator"')) continue;
    const nr = out.indexOf(`id="${shelf}-no-results"`, at);
    const close = out.indexOf('</div>', nr) + '</div>'.length;
    out = `${out.slice(0, close)}\n      <p class="shelf-curator">${escapeText(CURATED_BY)}</p>${out.slice(close)}`;
  }
  if (!out.includes('class="foot-curator"')) {
    out = out.replace(
      '<span>A research environment for biomedical AI, from the Baranzini Lab at UCSF.</span>',
      `$&\n      <span class="foot-curator">${escapeText(CURATED_BY)}</span>`,
    );
  }
  return out;
}

/** The real page as write-credits.mjs writes it from the committed catalogs. */
const WRITTEN = (() => {
  const r = regeneratePage(withCurators(REAL_PAGE), CATALOGS);
  return { ...r, credits: buildCredits(CATALOGS) };
})();

const sitesOf = (html, kind) => cardCreditSites(html).filter((s) => s.kind === kind);
const once = (html, from, to) => {
  const at = html.indexOf(from);
  assert.notEqual(at, -1, `the mutation's anchor "${from.slice(0, 60)}" must be on the page, or this proves nothing`);
  return html.slice(0, at) + to + html.slice(at + from.length);
};
const subjects = (problems) => problems.map((p) => p.subject);

// ---------------------------------------------------------------- credits.json

test('buildCredits writes the top level in the documented order', () => {
  const credits = buildCredits(TOY_CATALOGS);
  assert.deepEqual(Object.keys(credits), ['version', 'source', 'curated_by', 'made_by_lab', 'skills_commit', 'extensions', 'skills']);
  assert.equal(credits.version, CREDITS_VERSION);
  assert.equal(credits.version, 1);
  assert.equal(credits.source, 'https://biorouter.ucsf.edu/baam');
  assert.equal(credits.curated_by, CURATED_BY);
  assert.equal(credits.made_by_lab, MADE_BY_LAB);
  assert.equal(credits.skills_commit, 'c'.repeat(40));
});

test('an entry, and every list item in it, keeps the documented key order', () => {
  const e = buildCredits(TOY_CATALOGS).skills['toy-bundle'];
  assert.deepEqual(Object.keys(e), ENTRY_KEYS);
  for (const s of e.sources) assert.deepEqual(Object.keys(s), SOURCE_KEYS);
  for (const i of e.ideas) assert.deepEqual(Object.keys(i), IDEA_KEYS);
  for (const d of e.data_and_services) assert.deepEqual(Object.keys(d), DATA_KEYS);
  for (const p of e.people) assert.deepEqual(Object.keys(p), PERSON_KEYS);
  assert.deepEqual(Object.keys(e.earlier[0]), ['kind', 'version', 'origin', 'licence']);
  assert.deepEqual(Object.keys(e.earlier[1]), ['kind', ...IDEA_KEYS]);
});

test('an entry says what the record says', () => {
  const e = buildCredits(TOY_CATALOGS).skills['toy-bundle'];
  assert.equal(e.status, 'active');
  assert.equal(e.origin, 'adapted');
  assert.equal(e.line, creditLine(ADAPTED_BUNDLE, { catalog: 'skills' }));
  assert.equal(e.line, 'Adapted from toys by Some One (MIT).');
  assert.match(e.summary, /^Includes text or code from the sources below/);
  assert.equal(e.licence, 'Apache-2.0 AND MIT');
  // Every upstream, in record order, the dependency included.
  assert.deepEqual(e.sources.map((s) => s.name), ['toys', 'evil']);
  assert.deepEqual(e.sources[0], {
    name: 'toys', url: 'https://github.com/someone/toys', pin: 'commit 0123456', licence: 'MIT', credit: 'Some One',
    people: ['Some One'], use: ['text'], what: 'Two reference files.', owner: 'external', via: ['other/origin'],
  });
  assert.equal(e.sources[1].url, null, 'a javascript: url is dropped, never published');
  assert.equal(e.sources[1].pin, 'version 2.0.1');
  // Current ideas only; the earlier one sits in `earlier`.
  assert.deepEqual(e.ideas.map((i) => i.name), ['Idea Now']);
  assert.deepEqual(e.earlier.map((x) => x.kind), ['version', 'idea']);
  assert.deepEqual(e.earlier[0], { kind: 'version', version: '1.0.0', origin: 'in-house', licence: 'Apache-2.0' });
  assert.equal(e.earlier[1].url, null, 'a data: url is dropped, never published');
  assert.equal(e.data_and_services[0].name, 'PubMed');
  assert.deepEqual(e.people, [{ name: 'Wanjun Gu', affiliation: 'Baranzini Lab, UCSF', role: 'curator' }]);
  // Active components written for Biorouter, by name (the folder when unnamed); never a removed one.
  assert.deepEqual(e.written_for_biorouter, ['Written Here', 'idea-shaped']);
  assert.equal(e.moved_to, null);
  assert.equal(e.licence_files, `https://github.com/BaranziniLab/biorouter-skills/blob/${'c'.repeat(40)}/toy-bundle/LICENSE-UPSTREAM`);
  assert.equal(e.record, `https://github.com/BaranziniLab/biorouter-skills/blob/${'c'.repeat(40)}/README.md#provenance-and-credits`);
});

test('an extension entry links its NOTICE at its release tag and has no full record yet', () => {
  const e = buildCredits(TOY_CATALOGS).extensions.toyagent;
  assert.equal(e.line, 'Runs Toy MCP 1.0.0 by Toy <Org> (MIT); extension by the Baranzini Lab.');
  assert.equal(e.licence_files, 'https://github.com/BaranziniLab/ToyAgent/blob/v0.2.0/NOTICE');
  assert.equal(e.record, null);
  assert.deepEqual(e.written_for_biorouter, []);
});

test('a retired row keeps an entry and says where it went; an in-house bundle lists nothing as written for Biorouter', () => {
  const moved = creditsEntry({ ...EXTENSION, status: 'superseded_by:other', source: { ...EXTENSION.source, content_moved_to: 'other' } }, 'extensions');
  assert.equal(moved.moved_to, 'other');
  const inHouse = creditsEntry({ ...ADAPTED_BUNDLE, source: { origin: 'in-house' } }, 'skills', { skillsCommit: 'c'.repeat(40) });
  assert.equal(inHouse.line, MADE_BY_LAB);
  assert.deepEqual(inHouse.written_for_biorouter, []);
  const noCommit = creditsEntry(ADAPTED_BUNDLE, 'skills');
  assert.equal(noCommit.licence_files, null);
  assert.equal(noCommit.record, null);
});

test('the committed catalogs give an entry for every row, in catalog order', () => {
  const credits = buildCredits(CATALOGS);
  assert.deepEqual(Object.keys(credits.extensions), CATALOGS.extensions.items.map((r) => r.id));
  assert.deepEqual(Object.keys(credits.skills), CATALOGS.skills.items.map((r) => r.id));
  assert.equal(credits.skills_commit, CATALOGS.skills.source_commit);
  for (const entry of [...Object.values(credits.extensions), ...Object.values(credits.skills)]) {
    assert.deepEqual(Object.keys(entry), ENTRY_KEYS);
    for (const url of [entry.licence_files, entry.record, ...entry.sources.map((s) => s.url), ...entry.ideas.map((i) => i.url)]) {
      if (url !== null) assert.match(url, /^https?:\/\//);
    }
  }
});

test('serializeCredits is deterministic, two-space JSON with one trailing newline', () => {
  const a = serializeCredits(buildCredits(CATALOGS));
  const b = serializeCredits(buildCredits(structuredClone(CATALOGS)));
  assert.equal(a, b);
  assert.ok(a.endsWith('}\n') && !a.endsWith('\n\n'));
  assert.equal(a, JSON.stringify(JSON.parse(a), null, 2) + '\n');
  assert.deepEqual(CREDITS_PATHS, ['landing/credits.json', 'ui/desktop/src/components/baam/credits.fallback.json']);
});

// ---------------------------------------------------------------- the three guards

test('pinText pins a source by version, else short commit, a publication by its DOI', () => {
  assert.equal(pinText({ version: 'v1.2.3' }), 'version 1.2.3');
  assert.equal(pinText({ version: '0.0.1a8' }), 'version 0.0.1a8');
  assert.equal(pinText({ version: 'vNext' }), 'version vNext', 'a leading v is dropped only before a digit');
  assert.equal(pinText({ version: ' 2.0 ', commit: 'abcdef0123' }), 'version 2.0', 'the version wins over the commit');
  assert.equal(pinText({ commit: '0123456789abcdef0123456789abcdef01234567' }), 'commit 0123456');
  assert.equal(pinText({ commit: 'main' }), null, 'a branch name is not a pin');
  assert.equal(pinText({ commit: 'ABCDEF0' }), null);
  assert.equal(pinText({ kind: 'paper', doi: '10.1000/xyz', version: '3' }), 'DOI 10.1000/xyz');
  assert.equal(pinText({ kind: 'guideline', version: '2024' }), null, 'a guideline without a DOI is named, not pinned');
  assert.equal(pinText({}), null);
  assert.equal(pinText(null), null);
});

test('httpUrl keeps only http(s) URLs a page can show as a link', () => {
  assert.equal(httpUrl('https://example.org/a?b=1#c'), 'https://example.org/a?b=1#c');
  assert.equal(httpUrl('HTTP://EXAMPLE.ORG'), 'HTTP://EXAMPLE.ORG');
  assert.equal(httpUrl('  https://example.org  '), 'https://example.org');
  for (const bad of [
    'javascript:alert(1)',
    'JavaScript:alert(1)',
    ' javascript:alert(1)',
    'data:text/html,<script>alert(1)</script>',
    'data:image/png;base64,AAAA',
    'vbscript:msgbox',
    'ftp://example.org',
    '//example.org',
    '/relative/path',
    'https://example.org/"onmouseover="x',
    "https://example.org/'x",
    'https://example.org/<b>',
    'https://exa mple.org',
    'https://',
    '',
    null,
    undefined,
    42,
    { href: 'https://example.org' },
  ]) {
    assert.equal(httpUrl(bad), null, `${JSON.stringify(bad)} must not become a link`);
  }
});

test('escapeText makes text of anything, markup included', () => {
  assert.equal(escapeText('<img src=x onerror="window.__pwned=1">'), '&lt;img src=x onerror="window.__pwned=1"&gt;');
  assert.equal(escapeText('A & B'), 'A &amp; B');
  assert.equal(escapeText('&amp;'), '&amp;amp;', 'an entity in a record is text, so it is escaped again');
  assert.equal(escapeText(null), '');
  assert.equal(escapeText(undefined), '');
  assert.equal(escapeText(12), '12');
});

// ---------------------------------------------------------------- writing the page

test('the real page has one credit site per card: every extension and every skill card, featured included', () => {
  const sites = cardCreditSites(REAL_PAGE);
  const ext = sites.filter((s) => s.kind === 'extensions');
  const skills = sites.filter((s) => s.kind === 'skills');
  assert.equal(ext.length, (REAL_PAGE.match(/<div class="ext-card[\s"]/g) ?? []).length);
  assert.ok(ext.length >= 20, `${ext.length} extension cards`);
  assert.ok(skills.length >= 100, `${skills.length} skill cards`);
  assert.equal(skills.filter((s) => REAL_PAGE.slice(s.start, s.start + 40).includes('is-feat')).length, 3);
  for (const s of sites) assert.ok(s.id, `the card at ${s.start} has an id`);
});

test('writeCreditsHtml writes one line per card and is idempotent on the real page', () => {
  const page = withCurators(REAL_PAGE);
  const credits = buildCredits(CATALOGS);
  const a = writeCreditsHtml(page, credits);
  assert.deepEqual(a.problems, []);
  const b = writeCreditsHtml(a.html, credits);
  assert.deepEqual(b.problems, []);
  assert.ok(b.html === a.html, 'a second write changes nothing');
  for (const s of cardCreditSites(a.html)) {
    assert.equal(s.credits.length, 1, `${s.kind} ${s.id} carries one line`);
    assert.equal(s.credits[0].inner, escapeText(credits[s.kind][s.id].line));
    assert.equal(a.html.slice(s.descEnd, s.credits[0].start), `\n${s.indent}`, `${s.id}'s line is the next element after its description, indented like it`);
  }
  assert.deepEqual(creditPageProblems(a.html, credits), []);
});

test('writeCreditsHtml escapes the line and leaves every byte outside the generated elements alone', () => {
  const page = withCurators(REAL_PAGE);
  const credits = buildCredits(CATALOGS);
  const site = sitesOf(page, 'extensions')[0];
  const hostile = structuredClone(credits);
  hostile.extensions[site.id].line = '<img src=x onerror="window.__pwned=1"> & more';
  const { html, problems } = writeCreditsHtml(page, hostile);
  assert.deepEqual(problems, []);
  assert.ok(html.includes('<p class="item-credit">&lt;img src=x onerror="window.__pwned=1"&gt; &amp; more</p>'));
  assert.ok(!html.includes('<img src=x'), 'no markup from a record reaches the page');
  const strip = (h) =>
    h
      .replace(/\n[ \t]*<p class="item-credit">[\s\S]*?<\/p>/g, '')
      .replace(/<p class="shelf-curator">[\s\S]*?<\/p>/g, '<p class="shelf-curator"></p>')
      .replace(/<span class="foot-curator">[\s\S]*?<\/span>/g, '<span class="foot-curator"></span>');
  assert.ok(strip(html) === strip(page), 'only the generated elements changed');
});

test('writeCreditsHtml repairs a doubled, a misplaced and a changed line, and rewrites the curator text', () => {
  const credits = buildCredits(CATALOGS);
  let page = WRITTEN.html;
  const [ext] = sitesOf(page, 'extensions');
  const skill = sitesOf(page, 'skills')[5];
  const extLine = page.slice(ext.credits[0].start, ext.credits[0].end);
  // Doubled: a second copy of the extension card's line after the first.
  page = page.slice(0, ext.credits[0].end) + `\n        ${extLine}` + page.slice(ext.credits[0].end);
  // Changed: the skill card's line says something else.
  const s2 = sitesOf(page, 'skills')[5];
  page = page.slice(0, s2.credits[0].start) + '<p class="item-credit">Made by the Baranzini Lab.</p>' + page.slice(s2.credits[0].end);
  // A curator line with the wrong words.
  page = once(page, `<span class="foot-curator">${escapeText(CURATED_BY)}</span>`, '<span class="foot-curator">Curated by someone.</span>');
  assert.notDeepEqual(creditPageProblems(page, credits), []);
  const { html, problems } = writeCreditsHtml(page, credits);
  assert.deepEqual(problems, []);
  assert.ok(html === WRITTEN.html, 'the repaired page is the page as written');
  assert.equal(sitesOf(WRITTEN.html, 'skills')[5].id, skill.id);
});

/** The page with card `s`'s credit line moved to just before its description, on a line of its own. */
const lineBeforeDesc = (html, s) => {
  const line = html.slice(s.credits[0].start, s.credits[0].end);
  const without = html.slice(0, s.descEnd) + html.slice(s.credits[0].end);
  const descStart = without.lastIndexOf('<p class="', s.descEnd - 1);
  return without.slice(0, descStart) + `${line}\n${s.indent}` + without.slice(descStart);
};

test('writeCreditsHtml moves a line placed before the description back after it, without repeating text', () => {
  const credits = buildCredits(CATALOGS);
  for (const s of [sitesOf(WRITTEN.html, 'extensions')[2], sitesOf(WRITTEN.html, 'skills')[9]]) {
    const page = lineBeforeDesc(WRITTEN.html, s);
    assert.ok(page !== WRITTEN.html, 'the mutation must apply');
    const moved = sitesOf(page, s.kind).find((x) => x.start === s.start);
    assert.ok(moved.credits[0].start < moved.descEnd, 'the mutant puts the line before the description');
    const { html, problems } = writeCreditsHtml(page, credits);
    assert.deepEqual(problems, []);
    assert.ok(html === WRITTEN.html, `card ${s.id}: the repaired page is the page as written`);
  }
});

test('writeCreditsHtml keeps the line after the description and drops a second one before it', () => {
  const credits = buildCredits(CATALOGS);
  const s = sitesOf(WRITTEN.html, 'skills')[12];
  const line = WRITTEN.html.slice(s.credits[0].start, s.credits[0].end);
  const descStart = WRITTEN.html.lastIndexOf('<p class="', s.descEnd - 1);
  const page = WRITTEN.html.slice(0, descStart) + `${line}\n${s.indent}` + WRITTEN.html.slice(descStart);
  const { html, problems } = writeCreditsHtml(page, credits);
  assert.deepEqual(problems, []);
  assert.ok(html === WRITTEN.html, 'the repaired page is the page as written');
});

test('writeCreditsHtml refuses a shelf without its hand-placed curator element rather than inventing one', () => {
  const credits = buildCredits(CATALOGS);
  const page = once(WRITTEN.html, `<p class="shelf-curator">${escapeText(CURATED_BY)}</p>`, '');
  const { problems } = writeCreditsHtml(page, credits);
  assert.ok(subjects(problems).some((s) => s.startsWith('shelf:')), JSON.stringify(problems));
});

// ---------------------------------------------------------------- PV14, the page half

test('the page as written passes the page half of PV14, so the mutants below start clean', () => {
  assert.deepEqual(WRITTEN.problems, []);
  assert.deepEqual(creditPageProblems(WRITTEN.html, WRITTEN.credits), []);
  assert.deepEqual(handWrittenCredit(WRITTEN.html), []);
});

/** A mutant of the written page and the subject creditPageProblems must report for it. */
function caught(name, mutate, expected) {
  test(`PV14 catches ${name}`, () => {
    const page = mutate(WRITTEN.html);
    assert.ok(page !== WRITTEN.html, 'the mutation must apply, or this proves nothing');
    const problems = creditPageProblems(page, WRITTEN.credits);
    const want = typeof expected === 'function' ? expected(page) : expected;
    assert.ok(
      subjects(problems).some((s) => (want instanceof RegExp ? want.test(s) : s === want)),
      `expected ${want}, got ${JSON.stringify(problems, null, 1)}`,
    );
  });
}

const firstExt = () => sitesOf(WRITTEN.html, 'extensions')[0];
const extById = (id) => sitesOf(WRITTEN.html, 'extensions').find((s) => s.id === id) ?? firstExt();
const skillAt = (k) => sitesOf(WRITTEN.html, 'skills')[k];
const replaceSpan = (html, start, end, to) => html.slice(0, start) + to + html.slice(end);

caught(
  'a changed line (CDWAgent says "Made by the Baranzini Lab.")',
  (h) => {
    const s = extById('cdwagent');
    return replaceSpan(h, s.credits[0].start, s.credits[0].end, '<p class="item-credit">Made by the Baranzini Lab.</p>');
  },
  `card:extensions:${extById('cdwagent').id}`,
);
caught(
  'a missing line on a skill card',
  (h) => {
    const s = skillAt(10);
    return replaceSpan(h, s.descEnd, s.credits[0].end, '');
  },
  `card:skills:${skillAt(10).id}`,
);
caught(
  'a doubled line on an extension card',
  (h) => {
    const s = firstExt();
    return replaceSpan(h, s.credits[0].end, s.credits[0].end, `\n${s.indent}${h.slice(s.credits[0].start, s.credits[0].end)}`);
  },
  `card:extensions:${firstExt().id}`,
);
caught(
  'a misplaced line (moved below the card footer)',
  (h) => {
    const s = skillAt(20);
    const line = h.slice(s.credits[0].start, s.credits[0].end);
    const without = replaceSpan(h, s.descEnd, s.credits[0].end, '');
    const footer = '<div class="skill-footer">';
    const footerAt = without.indexOf(footer, s.start) + footer.length;
    return replaceSpan(without, footerAt, footerAt, line);
  },
  `card:skills:${skillAt(20).id}`,
);
caught('a line placed before the description on an extension card', (h) => lineBeforeDesc(h, extById('spokeagent')), `card:extensions:${extById('spokeagent').id}`);
caught('a line placed before the description on a skill card', (h) => lineBeforeDesc(h, skillAt(15)), `card:skills:${skillAt(15).id}`);
caught(
  'a featured card whose line differs from its grid twin',
  (h) => {
    const s = sitesOf(h, 'skills').find((x) => h.slice(x.start, x.start + 40).includes('is-feat'));
    return replaceSpan(h, s.credits[0].start, s.credits[0].end, '<p class="item-credit">From somewhere.</p>');
  },
  /^card:skills:/,
);
caught('a missing shelf curator line', (h) => once(h, `<p class="shelf-curator">${escapeText(CURATED_BY)}</p>`, ''), 'shelf:extensions-section');
caught(
  'the skills shelf without its curator line',
  (h) => {
    const at = h.lastIndexOf(`<p class="shelf-curator">${escapeText(CURATED_BY)}</p>`);
    return replaceSpan(h, at, at + `<p class="shelf-curator">${escapeText(CURATED_BY)}</p>`.length, '');
  },
  'shelf:skills-section',
);
caught(
  'a shelf curator line with other words',
  (h) => once(h, `<p class="shelf-curator">${escapeText(CURATED_BY)}</p>`, '<p class="shelf-curator">Curated by the lab.</p>'),
  'shelf:extensions-section',
);
caught(
  'a doubled shelf curator line',
  (h) => once(h, `<p class="shelf-curator">${escapeText(CURATED_BY)}</p>`, `<p class="shelf-curator">${escapeText(CURATED_BY)}</p><p class="shelf-curator">${escapeText(CURATED_BY)}</p>`),
  'shelf:extensions-section',
);
caught(
  "the footer's curator line with other words",
  (h) => once(h, `<span class="foot-curator">${escapeText(CURATED_BY)}</span>`, '<span class="foot-curator">Curated.</span>'),
  'footer',
);
caught('a missing footer curator line', (h) => once(h, `<span class="foot-curator">${escapeText(CURATED_BY)}</span>`, ''), 'footer');
caught(
  'a curator line on a card',
  (h) => {
    const s = skillAt(3);
    return replaceSpan(h, s.credits[0].end, s.credits[0].end, `<span class="foot-curator">${escapeText(CURATED_BY)}</span>`);
  },
  'stray',
);
// A credit element outside a card is not the generator's, wherever it sits and
// whatever it says: the stray count is page-wide, as it is for the curator
// classes, and its wording is read as hand-written page text. Before this, the
// text scan skipped every <p class="item-credit"> on the page and the stray
// count looked inside cards only, so each of these passed.
// [where, anchor, the element goes 'before' or 'after' the anchor, its text]
const OUTSIDE_A_CARD = [
  ['a shelf header (the extensions group head)', '<div class="group-head"><h2>Extensions</h2>', 'before', 'Adapted from X.'],
  ['a shelf header (the skills shelf bar)', '<div class="filter-chips" id="skill-chips"', 'before', 'Adapted from X.'],
  ['the footer', `<span class="foot-curator">${escapeText(CURATED_BY)}</span>`, 'after', escapeText(MADE_BY_LAB)],
  ['a detail template (the workflow card panel)', '<dl class="detail-list">', 'after', 'Adapted from X.'],
  ['the moved list', '<details class="baam-moved" id="baam-moved">', 'after', 'Adapted from X.'],
];
for (const [where, anchor, side, text] of OUTSIDE_A_CARD) {
  const el = `<p class="${CARD_CREDIT_CLASS}">${text}</p>`;
  const insert = (h) => once(h, anchor, side === 'after' ? anchor + el : el + anchor);
  caught(`an item-credit with credit wording in ${where} (stray)`, insert, 'stray');
  caught(`an item-credit with credit wording in ${where} (text)`, insert, /^text:.*(?:Adapted from X\.|Made by the Baranzini Lab)/);
}
caught(
  'an item-credit with no credit wording outside a card (stray)',
  (h) => once(h, '<dl class="detail-list">', `<dl class="detail-list"><p class="${CARD_CREDIT_CLASS}">Nothing here.</p>`),
  'stray',
);

test('a curator element outside every shelf and the footer is read as page text', () => {
  // The moved list sits after the last shelf and before the footer. (One inside
  // a shelf, a panel included, is that shelf's second line and fails "shelf:".)
  const anchor = '<details class="baam-moved" id="baam-moved">';
  const page = once(WRITTEN.html, anchor, `${anchor}<p class="shelf-curator">${escapeText(CURATED_BY)}</p>`);
  assert.ok(subjects(creditPageProblems(page, WRITTEN.credits)).includes('stray'));
  assert.ok(handWrittenCredit(page).some((hit) => hit.includes('Curated and organized by')), JSON.stringify(handWrittenCredit(page)));
});

test('a page without the landmarks still exempts its credit elements from the text scan', () => {
  // PV14 fails such a page as "page"; the text scan must not then flood with
  // every generated line.
  const page = WRITTEN.html.replace('id="baam-moved"', 'id="baam-gone"');
  assert.deepEqual(handWrittenCredit(page), []);
});

caught(
  'an item-credit class on another element',
  (h) => {
    const s = skillAt(4);
    const footerAt = h.indexOf('<div class="skill-footer">', s.start);
    return replaceSpan(h, footerAt, footerAt + '<div class="skill-footer">'.length, `<div class="skill-footer ${CARD_CREDIT_CLASS}">`);
  },
  `card:skills:${skillAt(4).id}`,
);
caught(
  'credit wording written by hand in a description',
  (h) => {
    const s = skillAt(7);
    const close = h.lastIndexOf('</p>', s.descEnd);
    return replaceSpan(h, close, close, ' Adapted from X.');
  },
  /^text:.*Adapted from X\./,
);
caught(
  'credit wording written by hand in a heading',
  (h) => once(h, '<h2>Biomedical analysis skills</h2>', '<h2>Biomedical analysis skills</h2> <span class="sub-note">Made by the Baranzini Lab</span>'),
  /^text:.*Made by the Baranzini Lab/,
);
caught('a card with no entry in credits.json', (h) => h.replace(/releases\/download\/skill-([a-z0-9-]+)\/\1\.zip/, 'releases/download/skill-not-a-row/not-a-row.zip'), 'card:skills:not-a-row');

test('a hand-written phrase in a comment, a script or an attribute is not page text', () => {
  const page = once(
    WRITTEN.html,
    '<footer class="site-footer">',
    '<!-- Adapted from X. --><script>var s = "Made by the Baranzini Lab";</script><footer class="site-footer" data-note="Adapted from Y">',
  );
  assert.deepEqual(handWrittenCredit(page), []);
});

// A browser shows each of these as the phrase itself, so the scan must read them
// the way the browser does, not only "&amp;".
for (const [name, phrase] of [
  ['a numeric space', 'Adapted&#32;from X.'],
  ['a hexadecimal space', 'Adapted&#x20;from X.'],
  ['a no-break space', 'Adapted&nbsp;from X.'],
  ['a numeric no-break space', 'Made&#160;by the Baranzini Lab'],
  ['a letter written as a reference', 'Ad&#97;pted from X.'],
  ['a soft hyphen inside a word', 'Adap&shy;ted from X.'],
]) {
  caught(
    `credit wording behind ${name}`,
    (h) => {
      const s = skillAt(7);
      const close = h.lastIndexOf('</p>', s.descEnd);
      return replaceSpan(h, close, close, ` ${phrase}`);
    },
    /^text:.*(?:from X\.|Baranzini Lab)/,
  );
}

test('a reference that shows as its own characters is read as those characters, once', () => {
  // "&amp;#32;" shows as the literal text "&#32;", not as a space, so the words
  // either side of it are not the phrase.
  const page = once(WRITTEN.html, '<footer class="site-footer">', '<p>Adapted&amp;#32;from X.</p><footer class="site-footer">');
  assert.deepEqual(handWrittenCredit(page), []);
});

test('an extension card states its id with data-registry-id when its download name carries a version', () => {
  // build-registry.mjs and baam.js take data-registry-id before the download
  // name, so the credit line must be looked up under the same id.
  const s = extById('cdwagent');
  const span = WRITTEN.html.slice(s.start, s.end);
  const dl = 'releases/download/v0.7.0/cdwagent.brxt';
  assert.ok(span.includes(dl), "cdwagent's card downloads cdwagent.brxt, or this proves nothing");
  const tagEnd = span.indexOf('>');
  const card = span.slice(0, tagEnd) + ' data-registry-id="cdwagent"' + span.slice(tagEnd).replace(dl, 'releases/download/v0.7.0/cdwagent-0.7.0.brxt');
  const page = replaceSpan(WRITTEN.html, s.start, s.end, card);
  const site = sitesOf(page, 'extensions').find((x) => x.start === s.start);
  assert.equal(site.id, 'cdwagent');
  assert.deepEqual(creditPageProblems(page, WRITTEN.credits), []);
  // Without the attribute the versioned name is the id, as the registry reads it.
  const bare = replaceSpan(WRITTEN.html, s.start, s.end, span.replace(dl, 'releases/download/v0.7.0/cdwagent-0.7.0.brxt'));
  assert.equal(sitesOf(bare, 'extensions').find((x) => x.start === s.start).id, 'cdwagent-0.7.0');
  // An empty attribute falls back to the download name, as build-registry's publishedId does.
  const empty = replaceSpan(WRITTEN.html, s.start, s.end, span.slice(0, tagEnd) + ' data-registry-id=""' + span.slice(tagEnd));
  assert.equal(sitesOf(empty, 'extensions').find((x) => x.start === s.start).id, 'cdwagent');
});

// ---------------------------------------------------------------- write-credits.mjs

test('regeneratePage leaves the page unchanged on a second run', () => {
  const first = regeneratePage(withCurators(REAL_PAGE), CATALOGS);
  assert.deepEqual(first.problems, []);
  const second = regeneratePage(first.html, CATALOGS);
  assert.deepEqual(second.problems, []);
  assert.equal(second.changed, false);
  assert.ok(second.html === first.html, 'the second run returns the same page');
  assert.deepEqual(second.sites, []);
});

test('regeneratePage names each site it changes, and refuses a page the checker would reject', () => {
  const s = skillAt(2);
  const page = replaceSpan(WRITTEN.html, s.credits[0].start, s.credits[0].end, '<p class="item-credit">Something else.</p>');
  const r = regeneratePage(page, CATALOGS);
  assert.equal(r.changed, true);
  assert.deepEqual(subjects(r.sites), [`card:skills:${s.id}`]);
  assert.ok(r.html === WRITTEN.html, 'the page is repaired to the page as written');

  const handWritten = once(WRITTEN.html, '<h2>Biomedical analysis skills</h2>', '<h2>Biomedical analysis skills, adapted from many</h2>');
  const refused = regeneratePage(handWritten, CATALOGS);
  assert.ok(subjects(refused.problems).some((x) => x.startsWith('text:')), JSON.stringify(refused.problems));
});

test('the committed page is what write-credits.mjs writes from the committed catalogs', () => {
  const r = regeneratePage(REAL_PAGE, CATALOGS);
  assert.deepEqual(r.problems, []);
  assert.deepEqual(r.sites, [], 'run node landing/scripts/write-credits.mjs');
  assert.equal(r.changed, false, 'run node landing/scripts/write-credits.mjs');
});

/** A page with every landmark pageRanges needs, one extension card and two skill cards. */
function toyPage({ skillAttrs = 'data-tags="a b" data-license="MIT" data-version="0.0.1" data-cat="core"', second = 'data-version="9.9.9" data-tags="x"' } = {}) {
  return [
    '<div id="extensions-section">',
    '<div class="ext-grid" id="extensions-grid">',
    '      <div class="ext-card" data-license="Apache-2.0">',
    '        <p class="ext-desc">A toy.</p>',
    '        <a href="https://github.com/BaranziniLab/ToyAgent/releases/download/v0.2.0/toyagent.brxt" class="brxt-chip">.brxt</a>',
    '      </div>',
    '</div>',
    '      <p class="shelf-curator">x</p>',
    '</div>',
    '<div id="workflows-section">',
    '      <p class="shelf-curator">x</p>',
    '</div>',
    '<div id="skills-section">',
    `      <div class="skill-card" ${skillAttrs}>`,
    '        <p class="skill-desc">A toy bundle.</p>',
    '        <a href="https://github.com/BaranziniLab/biorouter-skills/releases/download/skill-toy-bundle/toy-bundle.zip" class="skill-dl-btn">.zip</a>',
    '      </div>',
    `      <div class="skill-card is-feat" ${second}>`,
    '        <p class="skill-desc">The same toy bundle, featured.</p>',
    '        <a href="https://github.com/BaranziniLab/biorouter-skills/releases/download/skill-toy-bundle/toy-bundle.zip" class="skill-dl-btn">.zip</a>',
    '      </div>',
    '      <p class="shelf-curator">x</p>',
    '</div>',
    '<details class="baam-moved" id="baam-moved"></details>',
    '<footer class="site-footer"><div class="foot-brand"><span>Brand.</span>',
    '      <span class="foot-curator">x</span></div></footer>',
  ].join('\n');
}

test('writeSkillCardFields writes only data-version and data-license, in place, from the row', () => {
  const page = toyPage();
  const { html, problems, changes } = writeSkillCardFields(page, TOY_CATALOGS.skills.items);
  assert.deepEqual(problems, []);
  assert.ok(html.includes('<div class="skill-card" data-tags="a b" data-license="Apache-2.0 AND MIT" data-version="1.2.0" data-cat="core">'), html);
  // An attribute the tag lacked is added at its end; the others keep their order.
  assert.ok(html.includes('<div class="skill-card is-feat" data-version="1.2.0" data-tags="x" data-license="Apache-2.0 AND MIT">'), html);
  assert.deepEqual(subjects(changes), ['skill-card:toy-bundle', 'skill-card:toy-bundle', 'skill-card:toy-bundle', 'skill-card:toy-bundle']);
  // Nothing else moved: the extension card and everything outside the two start tags.
  const tags = /<div class="skill-card[^>]*>/g;
  assert.ok(html.replace(tags, '') === page.replace(tags, ''), 'nothing outside the two start tags changed');
  // And a second run has nothing to change.
  const again = writeSkillCardFields(html, TOY_CATALOGS.skills.items);
  assert.equal(again.html, html);
  assert.deepEqual(again.changes, []);
});

test('writeSkillCardFields escapes a row value and refuses a card it cannot write, leaving it untouched', () => {
  const odd = [{ ...ADAPTED_BUNDLE, licence: 'MIT" onclick="x' }];
  const escaped = writeSkillCardFields(toyPage(), odd);
  assert.ok(escaped.html.includes('data-license="MIT&quot; onclick=&quot;x"'));
  assert.ok(!escaped.html.includes('onclick="x"'));

  const page = toyPage();
  const noRow = writeSkillCardFields(page, []);
  assert.equal(noRow.html, page);
  assert.deepEqual(subjects(noRow.problems), ['skill-card:toy-bundle', 'skill-card:toy-bundle']);
  const noVersion = writeSkillCardFields(page, [{ ...ADAPTED_BUNDLE, version: '' }]);
  assert.ok(noVersion.problems.every((p) => /no version/.test(p.message)), JSON.stringify(noVersion.problems));
  assert.equal(noVersion.html, page, 'a card with a value it cannot write is left whole, not half written');
  const twice = writeSkillCardFields(toyPage({ skillAttrs: 'data-version="1" data-version="2" data-license="MIT"' }), TOY_CATALOGS.skills.items);
  assert.ok(twice.problems.some((p) => /data-version 2 times/.test(p.message)), JSON.stringify(twice.problems));
});

test('regeneratePage writes a toy page whole: lines, curator text and skill card fields', () => {
  const r = regeneratePage(toyPage(), TOY_CATALOGS);
  assert.deepEqual(r.problems, []);
  assert.equal((r.html.match(/<p class="item-credit">/g) ?? []).length, 3);
  assert.ok(r.html.includes('<p class="item-credit">Runs Toy MCP 1.0.0 by Toy &lt;Org&gt; (MIT); extension by the Baranzini Lab.</p>'));
  assert.ok(r.html.includes('<p class="item-credit">Adapted from toys by Some One (MIT).</p>'));
  assert.equal((r.html.match(new RegExp(`>${escapeText(CURATED_BY).replace(/[.()]/g, '\\$&')}<`, 'g')) ?? []).length, 4);
  assert.ok(r.sites.length >= 7, JSON.stringify(r.sites));
  assert.equal(regeneratePage(r.html, TOY_CATALOGS).changed, false);
});
