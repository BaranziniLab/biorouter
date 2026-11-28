// Tests for landing/scripts/check-catalog.mjs: the committed catalogs pass, the
// look-alike threshold is calibrated against the baseline, and each rule fails
// when the thing it guards is broken (one mutant per rule).
//
//   node --test landing/scripts/check-catalog.test.mjs

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  checkCatalogs,
  extensionRuleItems,
  loadCatalogs,
  mirroredBytes,
  movedEntries,
  readBuiltins,
  skillCardVersions,
} from './check-catalog.mjs';
import { DEFAULT_SIMILARITY_THRESHOLD, lookalikeScores, ruleD1, ruleD3 } from './lib/catalog-rules.mjs';

const SCRIPTS = dirname(fileURLToPath(import.meta.url));
const LANDING = join(SCRIPTS, '..');
const HTML = readFileSync(join(LANDING, 'baam.html'), 'utf8');
const REGISTRY = JSON.parse(readFileSync(join(LANDING, 'registry.json'), 'utf8'));
const CATALOGS = loadCatalogs();
const BUILTINS = readBuiltins();
// The extension id rename map, read here straight from the file the Rust
// crate compiles in, so the tests do not depend on the loader they test.
const RENAMES = JSON.parse(
  readFileSync(join(LANDING, '..', 'crates/biorouter/src/config/extension_renames.json'), 'utf8')
).renames.map(({ from, to }) => ({ from, to }));
const clone = (v) => structuredClone(v);

// `renames` left undefined means "the committed map", which checkCatalogs reads itself.
function check({ registry = REGISTRY, html = HTML, catalogs = CATALOGS, renames } = {}) {
  return checkCatalogs({ registry, html, catalogs, builtins: BUILTINS, renames });
}
const ext = (catalogs, id) => catalogs.extensions.items.find((r) => r.id === id);
const fails = (res, rule, subject) =>
  res.errors.some((e) => e.rule === rule && (subject === undefined || e.subject === subject || e.subject.startsWith(subject)));

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

// ---- The local id rename map (crates/biorouter/src/config/extension_renames.json)
// A row whose id or manifest_name the map renames records the new id in
// renamed_locally_to and in aliases, and no row records a rename the map lacks.

test('the gptragent row records the rename the map holds, and changes nothing else', () => {
  assert.ok(RENAMES.some((r) => r.from === 'gptragent' && r.to === 'gptr'), 'the map renames gptragent to gptr');
  const row = ext(CATALOGS, 'gptragent');
  assert.equal(row.renamed_locally_to, 'gptr');
  assert.deepEqual(row.aliases, ['gptr']);
  // The BAAM id, status and release stay: registry v2 cannot say which app
  // version can read a gptr bundle, so BAAM keeps serving gptragent.brxt.
  assert.equal(row.status, 'active');
  assert.equal(row.manifest_name, 'gptragent');
  assert.deepEqual(row.release, {
    tag: 'v0.1.2',
    asset: 'gptragent.brxt',
    sha256: '6de59047c1e87eed76fb9e67fbb75c9ac7c404eac7cbd75ed144091087fb1ea0',
  });
  assert.equal(CATALOGS.extensions.items.filter((r) => r.renamed_locally_to !== undefined).length, 1);
  assert.equal(CATALOGS.extensions.items.some((r) => r.id === 'gptr'), false, 'no gptr row before the turn-on pull request');
  assert.equal(REGISTRY.extensions.some((e) => e.id === 'gptr' || e.extension_name === 'gptr'), false);
});

test('mutant caught: a renamed row without renamed_locally_to', () => {
  const c = clone(CATALOGS);
  delete ext(c, 'gptragent').renamed_locally_to;
  assert.ok(fails(check({ catalogs: c }), 'rename', 'renamed-to:gptragent'));
});

test('mutant caught: a renamed row whose renamed_locally_to is not the map\'s new id', () => {
  const c = clone(CATALOGS);
  ext(c, 'gptragent').renamed_locally_to = 'gptr-agent';
  const res = check({ catalogs: c });
  assert.ok(fails(res, 'rename', 'renamed-to:gptragent'));
  // Claiming a rename the map does not hold is the second, separate failure.
  assert.ok(fails(res, 'rename', 'unmapped:gptragent'));
});

test('mutant caught: a renamed row without the new id in aliases', () => {
  const c = clone(CATALOGS);
  ext(c, 'gptragent').aliases = [];
  const res = check({ catalogs: c });
  assert.ok(fails(res, 'rename', 'alias:gptragent'));
  assert.equal(fails(res, 'rename', 'renamed-to:gptragent'), false, 'only the alias is missing');
});

test('mutant caught: a row that claims a rename the map lacks', () => {
  const c = clone(CATALOGS);
  ext(c, 'cdwagent').renamed_locally_to = 'cdw';
  ext(c, 'cdwagent').aliases.push('cdw');
  const res = check({ catalogs: c });
  assert.ok(fails(res, 'rename', 'unmapped:cdwagent'));
  assert.equal(fails(res, 'rename', 'renamed-to:cdwagent'), false);
});

test('mutant caught: the map renames a row the catalog does not record (the map gains a row)', () => {
  const res = check({ renames: [...RENAMES, { from: 'spokeagent', to: 'spoke' }] });
  assert.ok(fails(res, 'rename', 'renamed-to:spokeagent'));
  assert.ok(fails(res, 'rename', 'alias:spokeagent'));
});

test('a rename whose old id is a row\'s manifest_name, not its id, is held to the row too', () => {
  const renames = [...RENAMES, { from: 'spokekg', to: 'spoke' }];
  const c = clone(CATALOGS);
  ext(c, 'spokeagent').manifest_name = 'spokekg';
  const res = check({ catalogs: c, renames });
  assert.ok(fails(res, 'rename', 'renamed-to:spokeagent'));
  assert.ok(fails(res, 'rename', 'alias:spokeagent'));
  // Recorded on the row, the same rename passes.
  ext(c, 'spokeagent').renamed_locally_to = 'spoke';
  ext(c, 'spokeagent').aliases.push('spoke');
  assert.deepEqual(check({ catalogs: c, renames }).errors.filter((e) => e.rule === 'rename'), []);
});

test('the rename rule holds a superseded row too, so the turn-on keeps the record', () => {
  const c = clone(CATALOGS);
  const old = ext(c, 'gptragent');
  old.status = 'superseded_by:gptr';
  delete old.renamed_locally_to;
  assert.ok(fails(check({ catalogs: c }), 'rename', 'renamed-to:gptragent'));
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
