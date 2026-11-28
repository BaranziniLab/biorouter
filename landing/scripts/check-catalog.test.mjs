// Tests for landing/scripts/check-catalog.mjs: the committed catalogs pass, the
// look-alike threshold is calibrated against the baseline, and each rule fails
// when the thing it guards is broken (one mutant per rule). The last tests run
// landing/scripts/sync-skill-catalog.mjs against a temporary git repository.
//
//   node --test landing/scripts/check-catalog.test.mjs
//
// LANDING_ROOT points the tests at another copy of the site (its catalog/,
// baam.html and registry.json), which is how they are run against a tree that
// is not committed yet.

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync, execFileSync } from 'node:child_process';
import { cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  checkCatalogs,
  compareRulesMirror,
  extensionRuleItems,
  loadCatalogs,
  mirroredBytes,
  movedEntries,
  readBuiltins,
  ruleExtensionCredits,
  skillCardVersions,
} from './check-catalog.mjs';
import { CURATED_BY, DEFAULT_SIMILARITY_THRESHOLD, lookalikeScores, ruleD1, ruleD3 } from './lib/catalog-rules.mjs';
import { CREDITS_PATHS, buildCredits, cardCreditSites, curatorSites, serializeCredits } from './lib/credits.mjs';

const SCRIPTS = dirname(fileURLToPath(import.meta.url));
const LANDING = process.env.LANDING_ROOT ? resolve(process.env.LANDING_ROOT) : join(SCRIPTS, '..');
const HTML = readFileSync(join(LANDING, 'baam.html'), 'utf8');
const REGISTRY = JSON.parse(readFileSync(join(LANDING, 'registry.json'), 'utf8'));
const CATALOGS = loadCatalogs(join(LANDING, 'catalog'));
const BUILTINS = readBuiltins();
const clone = (v) => structuredClone(v);

function check({ registry = REGISTRY, html = HTML, catalogs = CATALOGS, creditsTexts } = {}) {
  return checkCatalogs({ registry, html, catalogs, builtins: BUILTINS, creditsTexts });
}
const ext = (catalogs, id) => catalogs.extensions.items.find((r) => r.id === id);
const fails = (res, rule, subject) =>
  res.errors.some((e) => e.rule === rule && (subject === undefined || e.subject === subject || e.subject.startsWith(subject)));
/** The rule failed with exactly this subject. */
const failsAt = (res, rule, subject) => res.errors.some((e) => e.rule === rule && e.subject === subject);

test('the committed catalogs agree with the page and pass every rule', () => {
  const res = check();
  assert.deepEqual(res.errors.map((e) => `[${e.rule}] ${e.message}`), []);
  assert.equal(CATALOGS.extensions.baseline_violations.length, 0, 'the extension baseline is empty at the end state');
  assert.equal(CATALOGS.skills.baseline_violations.length, 0, 'the skill baseline is empty at the end state');
});

test('the counts the check computes are the counts of the end state', () => {
  const { counts } = check();
  assert.equal(counts['tab-count-extensions'], REGISTRY.extensions.length);
  assert.equal(counts['tab-count-skills'], REGISTRY.skills.length);
});

test('every superseded or retired row is in the Moved and retired list once', () => {
  const moved = movedEntries(HTML);
  const inactive = [
    ...CATALOGS.extensions.items.filter((r) => r.status !== 'active'),
    ...CATALOGS.skills.items.filter((r) => r.status !== 'active'),
  ];
  assert.equal(moved.length, inactive.length);
});

// ---- Calibration of D3 against the baseline (plan 1.4) ---------------------
// The baseline is BAAM before the refresh, as WP-02 recorded it. At the frozen
// threshold the screen must flag the known extension look-alike pairs, and a
// known pair it does not flag must share a job (D1).
test('calibration: the frozen threshold flags the known extension pairs of the baseline', () => {
  const baseline = JSON.parse(readFileSync(join(SCRIPTS, 'fixtures/catalog/baseline-extensions.json'), 'utf8'));
  const items = extensionRuleItems(baseline).map((i) => ({ ...i, distinct_from: [] }));
  const rows = items.filter((i) => i.kind === 'extension');
  const flagged = new Set(ruleD3(rows, { threshold: CATALOGS.jobs.similarity_threshold }).map((f) => f.subject));
  assert.equal(CATALOGS.jobs.similarity_threshold, DEFAULT_SIMILARITY_THRESHOLD);
  for (const pair of ['literatureagent|structuredpapersearchagent', 'cellxgenecensusagent|singlecellomicsagent']) {
    assert.ok(flagged.has(`pair:${pair}`), `${pair} is flagged at ${CATALOGS.jobs.similarity_threshold}`);
  }
  // ncbiagent and unifiedpublicdatabaselookupagent score below the threshold,
  // so D1 has to catch them: they shared a job in the baseline.
  const score = lookalikeScores(rows).find((p) => [p.a.id, p.b.id].sort().join('|') === 'ncbiagent|unifiedpublicdatabaselookupagent');
  assert.ok(score.score < CATALOGS.jobs.similarity_threshold);
  assert.ok(ruleD1(items).some((f) => f.subject === 'job:bio.public-database-lookup'));
});

// ---- Mutants: each rule fails when its subject is broken --------------------

test('mutant caught: two active extensions own one job (D1)', () => {
  const c = clone(CATALOGS);
  ext(c, 'literatureagent').job = ext(c, 'biomcpagent').job;
  assert.ok(fails(check({ catalogs: c }), 'D1', 'job:'));
});

test('mutant caught: an extension claims a job Biorouter owns (D1)', () => {
  const c = clone(CATALOGS);
  ext(c, 'opencomputeruseagent').job = 'desktop.native-app-control';
  assert.ok(fails(check({ catalogs: c }), 'D1', 'builtin:desktop.native-app-control'));
});

test('mutant caught: a bundled adapter skill whose job is not .tool-usage (D1)', () => {
  const c = clone(CATALOGS);
  ext(c, 'gptragent').bundled_skills[0].job = 'r.live-session-usage';
  assert.ok(fails(check({ catalogs: c }), 'D1', 'tool-usage:'));
});

test('mutant caught: an alias that repeats another item\'s name (D2)', () => {
  const c = clone(CATALOGS);
  ext(c, 'primekgagent').aliases.push('SPOKEAgent');
  assert.ok(fails(check({ catalogs: c }), 'D2', 'name:spoke'));
});

test('mutant caught: a retired extension that no longer passes its name on (D2)', () => {
  const c = clone(CATALOGS);
  delete ext(c, 'ucsfhpcagent').retired.name_passes_to;
  assert.ok(fails(check({ catalogs: c }), 'D2', 'name:ucsfhpc'));
});

test('mutant caught: a look-alike pair with its boundary removed (D3)', () => {
  const c = clone(CATALOGS);
  ext(c, 'dnanexusagent').distinct_from = ext(c, 'dnanexusagent').distinct_from.filter((d) => d.id !== 'tamarindbioagent');
  ext(c, 'tamarindbioagent').distinct_from = ext(c, 'tamarindbioagent').distinct_from.filter((d) => d.id !== 'dnanexusagent');
  assert.ok(fails(check({ catalogs: c }), 'D3', 'pair:dnanexusagent|tamarindbioagent'));
});

test('mutant caught: a replacement that forgets the old id (D4)', () => {
  const c = clone(CATALOGS);
  const t = ext(c, 'unifiedpublicdatabaselookupagent');
  t.aliases = t.aliases.filter((a) => a !== 'ncbiagent');
  assert.ok(fails(check({ catalogs: c }), 'D4', 'alias:extension:ncbiagent'));
});

test('mutant caught: a redirect onto an item that is itself gone (D4, no chains)', () => {
  const c = clone(CATALOGS);
  const t = ext(c, 'literatureagent');
  t.status = 'retired';
  t.retired = { reason: 'mutant', use_instead: 'nothing' };
  assert.ok(fails(check({ catalogs: c }), 'D4', 'chain:extension:structuredpapersearchagent'));
});

test('mutant caught: a replacement card that does not name what it replaces (D4, page half)', () => {
  const r = clone(REGISTRY);
  const card = r.extensions.find((e) => e.id === 'cellxgenecensusagent');
  card.description = card.description.replace('Replaces SingleCellOmicsAgent. ', '');
  assert.ok(fails(check({ registry: r }), 'D4', 'card-names-old:singlecellomicsagent'));
});

test('mutant caught: a skill replacement whose card search misses the old id (D4, page half)', () => {
  const r = clone(REGISTRY);
  const card = r.skills.find((s) => s.id === 'frontend-design');
  card.keywords = card.keywords.filter((k) => k !== 'taste-skill');
  assert.ok(fails(check({ registry: r }), 'D4', 'card-tags-old:taste-skill'));
});

test('mutant caught: a required job whose owner retired, with nothing planned (G1)', () => {
  const c = clone(CATALOGS);
  const g = ext(c, 'gptragent');
  g.status = 'retired';
  g.retired = { reason: 'mutant', use_instead: 'nothing' };
  assert.ok(fails(check({ catalogs: c }), 'G1', 'gap:r.live-session'));
});

test('mutant caught: a superseded item missing from the Moved and retired list', () => {
  const html = HTML.replace(/<li data-moved-id="ncbiagent"[^\n]*\n/, '');
  assert.notEqual(html, HTML);
  assert.ok(fails(check({ html }), 'moved', 'count:extension:ncbiagent'));
});

test('mutant caught: the Moved and retired list names an item that is still active', () => {
  const html = HTML.replace('<ul>\n        <li data-moved-id', '<ul>\n        <li data-moved-id="spokeagent" data-moved-kind="extension" data-moved-to="">x</li>\n        <li data-moved-id');
  assert.notEqual(html, HTML);
  assert.ok(fails(check({ html }), 'moved', 'extra:extension:spokeagent'));
});

test('mutant caught: a card version that is not the catalog version', () => {
  const r = clone(REGISTRY);
  r.extensions.find((e) => e.id === 'bioroffice').version = 'v1.0.0';
  assert.ok(fails(check({ registry: r }), 'card', 'version:bioroffice'));
});

test('mutant caught: a download that is not the catalog release', () => {
  const r = clone(REGISTRY);
  const card = r.extensions.find((e) => e.id === 'idcagent');
  card.download = card.download.replace('/v0.2.0/', '/v0.1.0-brxt/');
  assert.ok(fails(check({ registry: r }), 'card', 'download:idcagent'));
});

test('mutant caught: a skill card whose data-version is not the catalog version', () => {
  const at = HTML.indexOf('skill-code-review/code-review.zip');
  const open = HTML.lastIndexOf('<div class="skill-card" ', at);
  const mutated = HTML.slice(0, open) + HTML.slice(open).replace(/data-version="[^"]*"/, 'data-version="1.0.0"');
  assert.equal(skillCardVersions(mutated).get('code-review')[0].version, '1.0.0');
  assert.ok(fails(check({ html: mutated }), 'card', 'version:code-review'));
});

test('mutant caught: a card that disagrees with the catalog about privacy', () => {
  const c = clone(CATALOGS);
  ext(c, 'gptragent').privacy = 'public';
  assert.ok(fails(check({ catalogs: c }), 'card', 'privacy:gptragent'));
});

test('mutant caught: a hand-written count that drifted', () => {
  const html = HTML.replace(/(id="count-adapters">)\d+/, '$1' + '31');
  assert.ok(fails(check({ html }), 'count', 'count-adapters'));
});

test('mutant caught: a baseline entry owned by a finished work package, and a stale one', () => {
  const c = clone(CATALOGS);
  ext(c, 'literatureagent').job = ext(c, 'biomcpagent').job;
  const subject = `job:${ext(c, 'biomcpagent').job}`;
  c.extensions.baseline_violations = [{ rule: 'D1', subject, wp: 'WP-01' }];
  assert.ok(check({ catalogs: c }).errors.some((e) => e.message.includes('which is marked done')));
  const d = clone(CATALOGS);
  d.extensions.baseline_violations = [{ rule: 'D1', subject: 'job:nothing.fails-here', wp: 'WP-99' }];
  assert.ok(fails(check({ catalogs: d }), 'baseline'));
});

test('mutant caught: jobs.json and the rules library disagree about the threshold', () => {
  const c = clone(CATALOGS);
  c.jobs.similarity_threshold = 0.4;
  assert.ok(fails(check({ catalogs: c }), 'jobs', 'rules-drift'));
});

test('the mirror is catalog.json plus one source_commit line', () => {
  const text = readFileSync(join(LANDING, 'catalog/skills.json'), 'utf8');
  const upstream = mirroredBytes(text);
  assert.ok(!upstream.includes('"source_commit"'));
  assert.equal(text.split('\n').length, upstream.split('\n').length + 1);
  assert.deepEqual(JSON.parse(upstream).items, JSON.parse(text).items);
  const c = clone(CATALOGS);
  delete c.skills.source_commit;
  assert.ok(fails(check({ catalogs: c }), 'mirror', 'source-commit'));
});

// ---- Format 2 ---------------------------------------------------------------

test('mutant caught: a catalog that is not format 2 (jobs.json stays format 1)', () => {
  assert.ok(!fails(check(), 'shape', 'format:'));
  const c = clone(CATALOGS);
  c.extensions.format = 1;
  assert.ok(failsAt(check({ catalogs: c }), 'shape', 'format:extensions'));
  const s = clone(CATALOGS);
  s.skills.format = 1;
  assert.ok(failsAt(check({ catalogs: s }), 'shape', 'format:skills'));
  const j = clone(CATALOGS);
  j.jobs.format = 2;
  assert.ok(failsAt(check({ catalogs: j }), 'shape', 'format:jobs'));
});

test('mutant caught: an extension origin that is not one of the four (the provenance row rules run, PV1)', () => {
  const c = clone(CATALOGS);
  ext(c, 'cdwagent').source.origin = 'native';
  assert.ok(failsAt(check({ catalogs: c }), 'PV1', 'origin:cdwagent'));
});

test('the provenance row rules do not run on skill rows here (biorouter-skills gates them)', () => {
  const c = clone(CATALOGS);
  const row = c.skills.items.find((r) => r.status === 'active');
  row.source.origin = 'native';
  const rowRules = (e) => /^PV\d+$/.test(e.rule) && Number(e.rule.slice(2)) <= 13;
  assert.ok(!check({ catalogs: c }).errors.some((e) => rowRules(e) && e.subject.includes(row.id)));
  // The page still shows the line the record gives, so PV14 sees the change.
  assert.ok(failsAt(check({ catalogs: c }), 'PV14', `card:skills:${row.id}`));
});

// ---- PV15, extension credits -------------------------------------------------

const upstream = (row, display) => row.source.upstreams.find((u) => u.display === display);

test('PV15 passes on the committed extension rows', () => {
  assert.deepEqual(ruleExtensionCredits(CATALOGS.extensions.items), []);
  assert.ok(!fails(check(), 'PV15'));
});

test('PV15 mutant: a third-party extension whose lead source it does not run (run)', () => {
  const c = clone(CATALOGS);
  upstream(ext(c, 'markitdownagent'), 'MarkItDown').use = ['dependency'];
  const res = check({ catalogs: c });
  assert.ok(failsAt(res, 'PV15', 'run:markitdownagent'));
  assert.ok(!failsAt(res, 'PV15', 'match:markitdownagent'), 'markitdown-mcp still supplies text, so a source matches');
});

test('PV15 mutant: an adapted extension that copies, bundles or launches nothing (match)', () => {
  const c = clone(CATALOGS);
  for (const u of ext(c, 'opencomputeruseagent').source.upstreams) u.use = ['dependency'];
  assert.ok(failsAt(check({ catalogs: c }), 'PV15', 'match:opencomputeruseagent'));
});

test('PV15 mutant: a launched program with neither a version nor a commit (entry)', () => {
  const c = clone(CATALOGS);
  const u = upstream(ext(c, 'playwrightagent'), 'Playwright MCP');
  delete u.version;
  delete u.commit;
  assert.ok(failsAt(check({ catalogs: c }), 'PV15', 'entry:playwrightagent:microsoft/playwright-mcp'));
});

test('PV15 mutant: a bundled program with no credit (entry)', () => {
  const c = clone(CATALOGS);
  delete upstream(ext(c, 'bioroffice'), 'OfficeCLI').credit;
  assert.ok(fails(check({ catalogs: c }), 'PV15', 'entry:bioroffice:'));
});

test('PV15 mutant: a service with no terms, and one whose terms wait on the owner (data)', () => {
  const c = clone(CATALOGS);
  const pubmed = ext(c, 'biomcpagent').source.data_and_services.find((d) => d.name === 'PubMed');
  delete pubmed.terms;
  assert.ok(failsAt(check({ catalogs: c }), 'PV15', 'data:biomcpagent:PubMed'));
  pubmed.terms = 'Pending the owner: read the terms page.';
  assert.ok(failsAt(check({ catalogs: c }), 'PV15', 'data:biomcpagent:PubMed'));
  pubmed.terms = 'Not read: the terms page did not load.';
  assert.ok(!failsAt(check({ catalogs: c }), 'PV15', 'data:biomcpagent:PubMed'), 'terms not read are a warning of the row rules');
});

test('PV15 mutant: a row that still carries third_party (field)', () => {
  const c = clone(CATALOGS);
  ext(c, 'zoteroagent').third_party = [];
  assert.ok(failsAt(check({ catalogs: c }), 'PV15', 'field:zoteroagent:third_party'));
});

test('PV15 reads only what it states: an in-house row and a hosted bridge pass', () => {
  const row = (origin, upstreams, data = []) => ({ id: 'x', status: 'active', source: { origin, upstreams, data_and_services: data } });
  const hosted = { display: 'S', repo: 'o/s', owner: 'external', use: ['text', 'hosted'], licence: 'MIT', credit: 'C', what: 'w' };
  assert.deepEqual(ruleExtensionCredits([row('in-house', [])]), []);
  assert.deepEqual(ruleExtensionCredits([row('third-party', [hosted])]), []);
  const onlyHosted = { ...hosted, use: ['hosted'] };
  assert.deepEqual(ruleExtensionCredits([row('third-party', [onlyHosted])]).map((f) => f.subject), ['match:x']);
  const labLaunched = { ...hosted, owner: 'lab', use: ['launched'], version: '1.0.0' };
  assert.deepEqual(
    ruleExtensionCredits([row('third-party', [labLaunched])]).map((f) => f.subject).sort(),
    ['match:x', 'run:x'],
    'a lab-owned program is not the outside work a third-party line credits'
  );
  const service = { kind: 'service', name: 'N', url: 'ftp://n.example', terms: 'Open.' };
  assert.deepEqual(ruleExtensionCredits([row('in-house', [], [service])]).map((f) => f.subject), ['data:x:N']);
  assert.ok(ruleExtensionCredits([row('in-house', [])]).every((f) => f.rule === 'PV15'));
});

// ---- PV14, the page and credits.json -------------------------------------------

const EXPECTED = buildCredits(CATALOGS);
const site = (kind, id) => cardCreditSites(HTML).find((s) => s.kind === kind && s.id === id);
/** The page with one span replaced. */
const splice = (html, start, end, text) => html.slice(0, start) + text + html.slice(end);
/** The page without the element at [start, end), its line break and indent included. */
const cut = (html, start, end) => {
  const lineStart = html.lastIndexOf('\n', start);
  return splice(html, html.slice(lineStart, start).trim() === '' ? lineStart : start, end, '');
};

test('PV14 passes on the committed page', () => {
  assert.ok(!fails(check(), 'PV14'), check().errors.filter((e) => e.rule === 'PV14').map((e) => e.message).join('\n'));
});

test('PV14 mutant: an extension card line that is not the generated line (card)', () => {
  const s = site('extensions', 'cdwagent');
  assert.equal(s.credits.length, 1);
  const html = splice(HTML, s.credits[0].start, s.credits[0].end, '<p class="item-credit">Made by the Baranzini Lab.</p>');
  assert.ok(failsAt(check({ html }), 'PV14', 'card:extensions:cdwagent'));
});

test('PV14 mutant: a skill card with its credit line deleted (card)', () => {
  const s = cardCreditSites(HTML).find((x) => x.kind === 'skills' && x.id && x.credits.length === 1);
  const html = cut(HTML, s.credits[0].start, s.credits[0].end);
  assert.ok(failsAt(check({ html }), 'PV14', `card:skills:${s.id}`));
});

test('PV14 mutant: a card credit line that does not follow the description (card)', () => {
  const s = site('extensions', 'spokeagent');
  const line = HTML.slice(s.credits[0].start, s.credits[0].end);
  let html = cut(HTML, s.credits[0].start, s.credits[0].end);
  const at = html.indexOf('<div class="ext-tags"', s.start);
  assert.ok(at > s.start && at < s.end);
  html = splice(html, html.indexOf('</div>', at) + '</div>'.length, html.indexOf('</div>', at) + '</div>'.length, line);
  assert.ok(failsAt(check({ html }), 'PV14', 'card:extensions:spokeagent'));
});

// The text between the end of the description and the start of the credit line
// is empty when the line sits BEFORE the description (slice(a, b) with a > b),
// so the order of the two must be checked as well as the gap between them.
test('PV14 mutant: a card credit line placed before the description (card)', () => {
  const s = site('extensions', 'spokeagent');
  const line = HTML.slice(s.credits[0].start, s.credits[0].end);
  let html = cut(HTML, s.credits[0].start, s.credits[0].end);
  const at = html.indexOf('<p class="ext-desc">', s.start);
  html = splice(html, at, at, `${line}\n`);
  assert.ok(failsAt(check({ html }), 'PV14', 'card:extensions:spokeagent'));
});

test('PV14 mutant: the skills shelf without its curator line (shelf)', () => {
  const [line] = curatorSites(HTML).shelves['skills-section'];
  const html = cut(HTML, line.start, line.end);
  const res = check({ html });
  assert.ok(failsAt(res, 'PV14', 'shelf:skills-section'));
  assert.ok(!failsAt(res, 'PV14', 'shelf:extensions-section'));
});

test('PV14 mutant: the footer curator line with other words (footer)', () => {
  const [line] = curatorSites(HTML).footer;
  const html = splice(HTML, line.start, line.end, '<span class="foot-curator">Curated by the lab.</span>');
  assert.ok(failsAt(check({ html }), 'PV14', 'footer'));
});

test('PV14 mutant: a curator line on a card (stray)', () => {
  const s = site('extensions', 'cdwagent');
  const html = splice(HTML, s.descEnd, s.descEnd, `<span class="foot-curator">${CURATED_BY}</span>`);
  assert.ok(failsAt(check({ html }), 'PV14', 'stray'));
});

test('PV14 mutant: credit wording written by hand into a card description (text)', () => {
  assert.ok(!fails(check(), 'PV14', 'text:'));
  const s = site('extensions', 'zoteroagent');
  const close = HTML.lastIndexOf('</p>', s.descEnd);
  const html = splice(HTML, close, close, ' Adapted from X.');
  assert.ok(fails(check({ html }), 'PV14', 'text:'));
});

test('PV14 mutant: credits.json and its app copy differ from the generator, or lack an active row (file, entry)', () => {
  const good = serializeCredits(EXPECTED);
  const both = (landing, app = good) => ({ [CREDITS_PATHS[0]]: landing, [CREDITS_PATHS[1]]: app });
  // Only the file and entry halves: the page half is the tests above.
  const pv14 = (texts) =>
    check({ creditsTexts: texts })
      .errors.filter((e) => e.rule === 'PV14' && /^(file|entry):/.test(e.subject))
      .map((e) => e.subject);
  assert.deepEqual(pv14(both(good)), []);
  assert.ok(!fails(check(), 'PV14', 'file:'), 'build-registry.mjs passes no texts, and none are compared');

  const changed = good.replace(/("line": ")[^"]*(")/, '$1Something else.$2');
  assert.notEqual(changed, good);
  assert.deepEqual(pv14(both(changed)), [`file:${CREDITS_PATHS[0]}`]);
  assert.deepEqual(pv14(both(good, changed)), [`file:${CREDITS_PATHS[1]}`]);
  assert.deepEqual(pv14(both(null, null)), [`file:${CREDITS_PATHS[0]}`, `file:${CREDITS_PATHS[1]}`]);
  assert.ok(pv14(both('{ not json')).includes(`file:${CREDITS_PATHS[0]}`));

  const active = CATALOGS.skills.items.find((r) => r.status === 'active').id;
  const parsed = JSON.parse(good);
  delete parsed.skills[active];
  const subjects = pv14(both(serializeCredits(parsed)));
  assert.ok(subjects.includes(`entry:skills:${active}`));
  assert.ok(subjects.includes(`file:${CREDITS_PATHS[0]}`));
  const inactive = CATALOGS.extensions.items.find((r) => r.status !== 'active').id;
  const old = JSON.parse(good);
  delete old.extensions[inactive];
  assert.ok(!pv14(both(serializeCredits(old))).includes(`entry:extensions:${inactive}`), 'an entry is required for active rows only');
});

test('PV14 reports a page it cannot read instead of throwing', () => {
  const html = HTML.replace('id="baam-moved"', 'id="baam-gone"');
  assert.ok(failsAt(check({ html }), 'PV14', 'page'));
});

// ---- The rules mirror --------------------------------------------------------

test('the rules mirror fails on a changed copy', () => {
  const rules = readFileSync(join(SCRIPTS, 'lib/catalog-rules.mjs'), 'utf8');
  assert.deepEqual(compareRulesMirror('x', rules, rules), []);
  const res = compareRulesMirror('x', rules.replace('RULES_VERSION = 1', 'RULES_VERSION = 2'), rules);
  assert.deepEqual(res.map((f) => [f.rule, f.subject]), [['mirror', 'rules']]);
  assert.deepEqual(compareRulesMirror('x', null, rules).map((f) => f.subject), ['rules'], 'a commit without the file fails');
});

// ---- sync-skill-catalog.mjs against a temporary git repository ----------------

const SYNC = join(SCRIPTS, 'sync-skill-catalog.mjs');
const RULES_TEXT = readFileSync(join(SCRIPTS, 'lib/catalog-rules.mjs'), 'utf8');

function skillsRepo(root) {
  const repo = join(root, 'skills');
  mkdirSync(join(repo, 'scripts/lib'), { recursive: true });
  const git = (...args) =>
    execFileSync('git', ['-C', repo, '-c', 'user.name=Test', '-c', 'user.email=test@example.invalid', '-c', 'commit.gpgsign=false', ...args], {
      encoding: 'utf8',
      maxBuffer: 64 * 1024 * 1024,
      stdio: ['ignore', 'pipe', 'pipe'],
    }).trim();
  git('init', '-q');
  const commit = (files, message) => {
    for (const [path, text] of Object.entries(files)) writeFileSync(join(repo, path), text);
    git('add', '-A');
    git('commit', '-q', '-m', message);
    return git('rev-parse', 'HEAD');
  };
  return { repo, commit };
}
const catalogText = (format) => `{\n  "format": ${format},\n  "description": "test",\n  "items": []\n}\n`;
function landingCopy(root) {
  const landing = join(root, 'site', 'landing');
  mkdirSync(join(landing, 'catalog'), { recursive: true });
  mkdirSync(join(landing, 'scripts/lib'), { recursive: true });
  writeFileSync(join(landing, 'catalog/skills.json'), 'untouched\n');
  writeFileSync(join(landing, 'scripts/lib/catalog-rules.mjs'), 'untouched\n');
  return landing;
}
const sync = (...args) => spawnSync(process.execPath, [SYNC, ...args], { encoding: 'utf8' });

test('sync refuses a catalog.json that is not format 2 and writes nothing', () => {
  const root = mkdtempSync(join(tmpdir(), 'sync-skill-catalog-'));
  try {
    const { repo, commit } = skillsRepo(root);
    const c1 = commit({ 'catalog.json': catalogText(1), 'scripts/lib/catalog-rules.mjs': RULES_TEXT }, 'format 1');
    const landing = landingCopy(root);
    const r = sync('--skills-repo', repo, '--commit', c1, '--mirror-only', '--landing-dir', landing);
    assert.equal(r.status, 1, r.stderr);
    assert.match(r.stderr, /"format": 1/);
    assert.equal(readFileSync(join(landing, 'catalog/skills.json'), 'utf8'), 'untouched\n');
    assert.equal(readFileSync(join(landing, 'scripts/lib/catalog-rules.mjs'), 'utf8'), 'untouched\n');
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test('sync writes the mirror and the rules copy of a format 2 catalog at its commit', () => {
  const root = mkdtempSync(join(tmpdir(), 'sync-skill-catalog-'));
  try {
    const { repo, commit } = skillsRepo(root);
    const c2 = commit({ 'catalog.json': catalogText(2), 'scripts/lib/catalog-rules.mjs': RULES_TEXT }, 'format 2');
    // A later commit must not be what is mirrored: the sync reads the commit it is given.
    commit({ 'catalog.json': catalogText(2).replace('"test"', '"later"'), 'scripts/lib/catalog-rules.mjs': `${RULES_TEXT}// later\n` }, 'later');
    const landing = landingCopy(root);
    const r = sync('--skills-repo', repo, '--commit', c2, '--mirror-only', '--landing-dir', landing);
    assert.equal(r.status, 0, r.stderr);
    const mirror = readFileSync(join(landing, 'catalog/skills.json'), 'utf8');
    assert.equal(mirror, `{\n  "source_commit": "${c2}",\n${catalogText(2).slice(2)}`);
    assert.equal(mirroredBytes(mirror), catalogText(2));
    assert.equal(readFileSync(join(landing, 'scripts/lib/catalog-rules.mjs'), 'utf8'), RULES_TEXT);
    assert.ok(!existsSync(join(landing, 'credits.json')), '--mirror-only runs no generator');
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test('sync creates the catalog and rules folders of an empty --landing-dir', () => {
  const root = mkdtempSync(join(tmpdir(), 'sync-skill-catalog-'));
  try {
    const { repo, commit } = skillsRepo(root);
    const c2 = commit({ 'catalog.json': catalogText(2), 'scripts/lib/catalog-rules.mjs': RULES_TEXT }, 'format 2');
    const landing = join(root, 'empty', 'landing');
    const r = sync('--skills-repo', repo, '--commit', c2, '--mirror-only', '--landing-dir', landing);
    assert.equal(r.status, 0, r.stderr);
    assert.equal(mirroredBytes(readFileSync(join(landing, 'catalog/skills.json'), 'utf8')), catalogText(2));
    assert.equal(readFileSync(join(landing, 'scripts/lib/catalog-rules.mjs'), 'utf8'), RULES_TEXT);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test('sync runs the two generators in order from the repository root and stops at the first failure', () => {
  const root = mkdtempSync(join(tmpdir(), 'sync-skill-catalog-'));
  try {
    const { repo, commit } = skillsRepo(root);
    const c2 = commit({ 'catalog.json': catalogText(2), 'scripts/lib/catalog-rules.mjs': RULES_TEXT }, 'format 2');
    const landing = landingCopy(root);
    const log = join(root, 'ran.log');
    const stub = (name, code) =>
      writeFileSync(
        join(landing, 'scripts', name),
        `import { appendFileSync } from 'node:fs';\nappendFileSync(${JSON.stringify(log)}, ${JSON.stringify(name)} + ' ' + process.cwd() + '\\n');\nprocess.exit(${code});\n`
      );
    stub('write-credits.mjs', 0);
    stub('build-registry.mjs', 0);
    const ok = sync('--skills-repo', repo, '--commit', c2, '--landing-dir', landing);
    assert.equal(ok.status, 0, ok.stderr);
    const cwd = realCwd(join(landing, '..'));
    assert.deepEqual(readFileSync(log, 'utf8').trim().split('\n'), [`write-credits.mjs ${cwd}`, `build-registry.mjs ${cwd}`]);

    rmSync(log);
    stub('write-credits.mjs', 3);
    const bad = sync('--skills-repo', repo, '--commit', c2, '--landing-dir', landing);
    assert.equal(bad.status, 3);
    assert.deepEqual(readFileSync(log, 'utf8').trim().split('\n'), [`write-credits.mjs ${cwd}`], 'build-registry.mjs does not run after a failure');
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
/** The directory as a child process reports it (macOS resolves /var to /private/var). */
function realCwd(dir) {
  return execFileSync(process.execPath, ['-e', 'process.stdout.write(process.cwd())'], { cwd: dir, encoding: 'utf8' });
}

test('check-catalog --skills-repo compares the rules copy at source_commit', () => {
  const root = mkdtempSync(join(tmpdir(), 'check-catalog-rules-'));
  try {
    const { repo, commit } = skillsRepo(root);
    const landing = landingCopy(root);
    for (const f of ['extensions.json', 'jobs.json']) cpSync(join(LANDING, 'catalog', f), join(landing, 'catalog', f));
    const mirrorErrors = (sha) => {
      const s = sync('--skills-repo', repo, '--commit', sha, '--mirror-only', '--landing-dir', landing);
      assert.equal(s.status, 0, s.stderr);
      const r = spawnSync(
        process.execPath,
        [join(SCRIPTS, 'check-catalog.mjs'), '--json', '--catalog', join(landing, 'catalog'), '--skills-repo', repo],
        { encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 }
      );
      assert.ok(r.status === 0 || r.status === 1, r.stderr);
      return JSON.parse(r.stdout).errors.filter((e) => e.rule === 'mirror').map((e) => e.subject);
    };
    const same = commit({ 'catalog.json': catalogText(2), 'scripts/lib/catalog-rules.mjs': RULES_TEXT }, 'verbatim rules');
    assert.deepEqual(mirrorErrors(same), []);
    const changed = commit({ 'scripts/lib/catalog-rules.mjs': RULES_TEXT.replace('RULES_VERSION = 1', 'RULES_VERSION = 2') }, 'changed rules');
    assert.deepEqual(mirrorErrors(changed), ['rules']);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
