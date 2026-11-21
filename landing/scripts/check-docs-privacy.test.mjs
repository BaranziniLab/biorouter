#!/usr/bin/env node --test
// A mutant suite for `check-docs-privacy.mjs`.
//
// The checker's only job is to fail. Running it once against a tree that
// already agrees proves nothing: a `checkDocsPrivacy` rewritten to `return []`
// passes that invocation, passes `just check-everything`, and passes the
// landing deploy gate — while the docs table it was written to police drifts
// freely. So the gate needs a gate: every mutation below is a way the table or
// the registry could be wrong, and each one must produce at least one failure.
//
// In-memory only. The mutants are string edits to copies of the real
// `docs.html` and `registry.json`, so nothing here writes to the tree and the
// suite is safe to run from `check-everything` and from CI.
import { readFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import assert from 'node:assert/strict';
import { checkDocsPrivacy } from './check-docs-privacy.mjs';

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const DOCS = readFileSync(join(ROOT, 'docs.html'), 'utf8');
const REGISTRY = JSON.parse(readFileSync(join(ROOT, 'registry.json'), 'utf8'));

const registryCopy = () => JSON.parse(JSON.stringify(REGISTRY));

/** The one edit every mutant needs: replace exactly once, and prove it landed. */
const replaceOnce = (haystack, needle, replacement) => {
  const at = haystack.indexOf(needle);
  assert.notEqual(at, -1, `the fixture no longer contains ${JSON.stringify(needle)} — update this suite`);
  assert.equal(
    haystack.indexOf(needle, at + needle.length),
    -1,
    `${JSON.stringify(needle)} appears more than once — the mutation would be ambiguous`
  );
  return haystack.slice(0, at) + replacement + haystack.slice(at + needle.length);
};

const ROW = {
  ucsfomop: '<tr><td><strong>UCSFOMOPAgent</strong></td><td>Private</td>',
  cdw: '<tr><td><strong>CDWAgent</strong></td><td>Private</td>',
  spoke: '<tr><td><strong>SPOKEAgent</strong></td><td>Public</td>',
  bioroffice: '<tr><td><strong>BiorOffice</strong></td><td>Public</td>',
};

/** The full `<tr>…</tr>` of a row, so a mutant can delete or duplicate it. */
const wholeRow = (docs, opener) => {
  const at = docs.indexOf(opener);
  assert.notEqual(at, -1, `the fixture no longer contains the row ${JSON.stringify(opener)}`);
  const end = docs.indexOf('</tr>', at);
  return docs.slice(at, end + '</tr>'.length);
};

const THEAD = '<thead><tr><th>Agent</th><th>Privacy</th><th>What it connects</th><th>Credentials</th></tr></thead>';
const SECTION = 'Extension agents in the marketplace';

/** The whole section — heading, prose and table — for the duplication mutants. */
const wholeSection = (docs) => {
  const at = docs.indexOf(`<h2>${SECTION}</h2>`);
  assert.notEqual(at, -1, 'the fixture no longer opens the section with an <h2>');
  const end = docs.indexOf('</table></div>', at);
  assert.notEqual(end, -1, 'the fixture no longer closes the section table with </table></div>');
  return docs.slice(at, end + '</table></div>'.length);
};

// The baseline. If this ever fails, the tree is wrong, not the suite.
test('the committed docs table agrees with the committed registry', () => {
  assert.deepEqual(checkDocsPrivacy(DOCS, REGISTRY), []);
});

/**
 * Every mutant: a name, the docs it hands the checker, the registry it hands
 * the checker, and a fragment the failure must contain — so a mutation cannot
 * be scored a catch by an unrelated complaint.
 */
const mutants = [
  // ── The tier itself disagrees ──────────────────────────────────────────
  {
    name: 'a private agent documented as Public',
    docs: (d) => replaceOnce(d, ROW.ucsfomop, ROW.ucsfomop.replace('<td>Private</td>', '<td>Public</td>')),
    expect: /UCSFOMOPAgent is documented as Public/,
  },
  {
    name: 'a public agent documented as Private',
    docs: (d) => replaceOnce(d, ROW.spoke, ROW.spoke.replace('<td>Public</td>', '<td>Private</td>')),
    expect: /SPOKEAgent is documented as Private/,
  },
  {
    name: 'the registry flips a tier under a table nobody updated',
    registry: (r) => {
      const ext = r.extensions.find((candidate) => candidate.id === 'cdwagent');
      ext.privacy = 'public';
      return r;
    },
    expect: /CDWAgent is documented as Private/,
  },
  {
    name: 'a tier spelled in lower case',
    docs: (d) => replaceOnce(d, ROW.cdw, ROW.cdw.replace('<td>Private</td>', '<td>private</td>')),
    expect: /expected exactly Private or Public/,
  },
  {
    name: 'an emptied tier cell',
    docs: (d) => replaceOnce(d, ROW.cdw, ROW.cdw.replace('<td>Private</td>', '<td></td>')),
    expect: /expected exactly Private or Public/,
  },

  // ── The column, and markup that only looks like the column ─────────────
  {
    name: 'the Privacy header deleted',
    docs: (d) => replaceOnce(d, '<th>Privacy</th>', ''),
    expect: /no Privacy column/,
  },
  {
    name: 'the Privacy header commented out',
    docs: (d) => replaceOnce(d, '<th>Privacy</th>', '<!-- <th>Privacy</th> -->'),
    expect: /no Privacy column/,
  },
  {
    name: 'the whole table commented out',
    docs: (d) => {
      const table = wholeSection(d);
      return replaceOnce(d, table, `<!-- ${table} -->`);
    },
    expect: /no longer has a "Extension agents in the marketplace" section/,
  },
  {
    name: 'a second Privacy header, disagreeing with the first',
    docs: (d) =>
      replaceOnce(d, THEAD, THEAD.replace('<th>Credentials</th>', '<th>Credentials</th><th>Privacy</th>')),
    expect: /more than one Privacy column|exactly one/,
  },
  {
    name: 'a second, conflicting copy of the whole section',
    docs: (d) => {
      const section = wholeSection(d);
      const conflicting = section.replace('<td>Private</td>', '<td>Public</td>');
      return replaceOnce(d, section, section + conflicting);
    },
    expect: /mentions "Extension agents in the marketplace" 2 times/,
  },

  // ── Rows that hide from a `<tr>`-literal reader ────────────────────────
  {
    name: 'a mis-tiered row wearing a class attribute',
    docs: (d) =>
      replaceOnce(
        d,
        ROW.bioroffice,
        ROW.bioroffice.replace('<tr>', '<tr class="marketplace-row">').replace('<td>Public</td>', '<td>Private</td>')
      ),
    expect: /BiorOffice is documented as Private/,
  },
  {
    name: 'a row one cell short, so its tier reads out of the wrong column',
    docs: (d) => replaceOnce(d, ROW.bioroffice, '<tr><td><strong>BiorOffice</strong></td><td>Public</td>'.replace('<td>Public</td>', '')),
    expect: /cells|column/i,
  },

  // ── Rows that vanish ───────────────────────────────────────────────────
  {
    name: 'a private row deleted outright',
    docs: (d) => replaceOnce(d, wholeRow(d, ROW.cdw), ''),
    expect: /CDWAgent/,
  },
  {
    name: 'a public row deleted outright',
    docs: (d) => replaceOnce(d, wholeRow(d, ROW.bioroffice), ''),
    expect: /BiorOffice/,
  },
  {
    name: 'a documented agent replaced by a duplicate of another',
    docs: (d) => replaceOnce(d, wholeRow(d, ROW.bioroffice), wholeRow(d, ROW.cdw)),
    expect: /BiorOffice|twice|more than once/i,
  },
  {
    name: 'a newly private registry extension the table never gained a row for',
    registry: (r) => {
      const ext = r.extensions.find((candidate) => candidate.id === 'ncbiagent');
      ext.privacy = 'private';
      return r;
    },
    expect: /does not list it/,
  },

  // ── The join between the two files ─────────────────────────────────────
  {
    name: 'a row naming an extension nobody publishes',
    docs: (d) => replaceOnce(d, '<strong>BiorOffice</strong>', '<strong>BiorOfficeX</strong>'),
    expect: /not in registry\.json/,
  },
  {
    name: 'a synthetic private extension colliding with a public one by alias',
    registry: (r) => {
      r.extensions.push({
        id: 'spoke-agent-shim',
        name: 'SPOKE Agent',
        privacy: 'private',
      });
      return r;
    },
    expect: /alias|collide|already/i,
  },
  {
    name: 'a registry entry with no tier at all',
    registry: (r) => {
      delete r.extensions.find((candidate) => candidate.id === 'cdwagent').privacy;
      return r;
    },
    expect: /declares no privacy tier/,
  },
  {
    name: 'an empty registry',
    registry: () => ({ extensions: [] }),
    expect: /no usable extensions|no extensions/i,
  },
];

for (const mutant of mutants) {
  test(`caught: ${mutant.name}`, () => {
    const docs = mutant.docs ? mutant.docs(DOCS) : DOCS;
    const registry = mutant.registry ? mutant.registry(registryCopy()) : registryCopy();
    const failures = checkDocsPrivacy(docs, registry);
    assert.ok(failures.length > 0, `mutation went undetected: ${mutant.name}`);
    assert.ok(
      failures.some((failure) => mutant.expect.test(failure)),
      `detected, but for the wrong reason. Wanted ${mutant.expect}, got:\n- ${failures.join('\n- ')}`
    );
  });
}
