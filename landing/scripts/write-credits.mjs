#!/usr/bin/env node
// Write the credit text into landing/baam.html from the BAAM catalogs.
//
//   node landing/scripts/write-credits.mjs          # rewrites landing/baam.html
//   node landing/scripts/write-credits.mjs --check  # writes nothing; exit 1 on a difference
//
// What it writes, and nothing else:
//
//   * every card's credit line, the <p class="item-credit"> that follows the
//     card's description (each .ext-card in #extensions-grid and each
//     .skill-card in #skills-section, the featured cards included), from
//     creditLine() in lib/catalog-rules.mjs by way of lib/credits.mjs;
//   * the text of the three shelf curator lines and the footer's curator line,
//     which are placed once by hand and kept equal to CURATED_BY here;
//   * each skill card's data-version and data-license, from its catalog row,
//     because they are row fields too and the skills mirror sync must leave a
//     page the catalog check accepts. Attribute order is kept and no other
//     attribute is touched. Extension card fields stay hand-maintained.
//
// It reads landing/catalog/extensions.json and skills.json directly, builds
// credits.json in memory (the same buildCredits() build-registry.mjs publishes),
// and refuses to write when a site cannot be written (a card with no
// description, no download link or no row; a shelf or the footer without its
// one curator element) or when the page it would write fails the page half of
// rule PV14 (creditPageProblems), hand-written credit wording included. The
// page is staged beside itself and renamed into place, so no reader sees half
// a page. Running it twice changes nothing.
//
// --check regenerates the page in memory, writes nothing, and exits 1 naming
// the first ten sites that differ (or every problem that stops a write). It is
// what tells the integrator, and CI, that the committed page carries the lines
// the committed catalogs give.
//
// Run it before build-registry.mjs: the registry reads the skill cards'
// data-version and data-license from this page.

import { readFileSync, realpathSync, renameSync, rmSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { buildCredits, cardCreditSites, creditPageProblems, curatorSites, writeCreditsHtml } from './lib/credits.mjs';

const LANDING = join(dirname(fileURLToPath(import.meta.url)), '..');
export const PAGE = join(LANDING, 'baam.html');
export const CATALOG_DIR = join(LANDING, 'catalog');
/** How many differing sites --check names before it stops listing. */
export const CHECK_LIST_MAX = 10;

/** The two catalogs the credits are built from, parsed. */
export function loadCatalogs(dir = CATALOG_DIR) {
  const read = (name) => JSON.parse(readFileSync(join(dir, name), 'utf8'));
  return { extensions: read('extensions.json'), skills: read('skills.json') };
}

/** An attribute value as HTML: & " < > escaped. */
const escapeAttr = (s) => String(s).replace(/&/g, '&amp;').replace(/"/g, '&quot;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
const SKILL_START = /^<div class="skill-card(?:\s[^"]*)?"[^>]*>/;
const SKILL_FIELDS = [
  ['data-version', 'version'],
  ['data-license', 'licence'],
];

/**
 * Every .skill-card start tag in #skills-section (featured cards included)
 * with data-version set to its row's `version` and data-license to its row's
 * `licence`. A card's row is the one whose id its download names. Attribute
 * order is kept and only those two values change; an attribute the tag lacks is
 * added at the end of the tag. Returns { html, problems, changes }: problems
 * are cards it cannot write (no download link, no row, a row with no value, an
 * attribute written twice), left untouched; changes name every value it
 * changed, as { subject: "skill-card:<id>", message }.
 */
export function writeSkillCardFields(html, skillRows) {
  const rows = new Map((Array.isArray(skillRows) ? skillRows : []).map((r) => [r?.id, r]));
  const problems = [];
  const changes = [];
  const edits = [];
  for (const site of cardCreditSites(html)) {
    if (site.kind !== 'skills') continue;
    const tag = SKILL_START.exec(html.slice(site.start))?.[0];
    if (!tag) continue;
    if (!site.id) {
      problems.push({ subject: `skill-card:${site.start}`, message: `the skill card at offset ${site.start} has no download link, so it has no row` });
      continue;
    }
    const subject = `skill-card:${site.id}`;
    const row = rows.get(site.id);
    if (!row) {
      problems.push({ subject, message: `the skill card ${site.id} has no row in landing/catalog/skills.json` });
      continue;
    }
    let next = tag;
    let ok = true;
    for (const [attr, field] of SKILL_FIELDS) {
      const value = typeof row[field] === 'string' ? row[field].trim() : '';
      if (!value) {
        problems.push({ subject, message: `the row ${site.id} has no ${field}, so the card's ${attr} cannot be written` });
        ok = false;
        continue;
      }
      const re = new RegExp(`(\\s${attr}=")([^"]*)(")`, 'g');
      const found = [...next.matchAll(re)];
      if (found.length > 1) {
        problems.push({ subject, message: `the skill card ${site.id} writes ${attr} ${found.length} times` });
        ok = false;
        continue;
      }
      const want = escapeAttr(value);
      if (found.length === 0) {
        next = next.replace(/>$/, ` ${attr}="${want}">`);
        changes.push({ subject, message: `${attr} added as "${want}"` });
      } else if (found[0][2] !== want) {
        changes.push({ subject, message: `${attr} "${found[0][2]}" becomes "${want}"` });
        next = next.replace(new RegExp(`(\\s${attr}=")([^"]*)(")`), (_m, a, _v, c) => `${a}${want}${c}`);
      }
    }
    if (ok && next !== tag) edits.push([site.start, site.start + tag.length, next]);
  }
  edits.sort((a, b) => b[0] - a[0]);
  let out = html;
  for (const [start, end, replacement] of edits) out = out.slice(0, start) + replacement + out.slice(end);
  return { html: out, problems, changes: problems.length ? changes.filter((c) => !problems.some((p) => p.subject === c.subject)) : changes };
}

/**
 * The page with every credit site and skill card field written from
 * `catalogs` ({ extensions, skills }, parsed). Returns
 * { html, problems, changed, sites, credits }:
 *
 *   problems  sites that stop a write: what writeCreditsHtml and
 *             writeSkillCardFields cannot write, and anything the written page
 *             still fails in creditPageProblems (hand-written credit wording
 *             among them). Each subject once.
 *   changed   whether the written page differs from `html`.
 *   sites     the sites that differ, in page order of kind (credit lines and
 *             curator lines, then skill card fields), as { subject, message }.
 */
export function regeneratePage(html, catalogs) {
  const credits = buildCredits({ extensions: catalogs?.extensions, skills: catalogs?.skills });
  const lines = writeCreditsHtml(html, credits);
  const fields = writeSkillCardFields(lines.html, catalogs?.skills?.items);
  const problems = [];
  const seen = new Set();
  const add = (p) => {
    if (seen.has(p.subject)) return;
    seen.add(p.subject);
    problems.push(p);
  };
  lines.problems.forEach(add);
  fields.problems.forEach(add);
  // The page this writes must pass the checker's page half. A site the writer
  // left wrong is a defect found here, before anything reaches the disk.
  creditPageProblems(fields.html, credits).forEach(add);
  const sites = [
    ...creditPageProblems(html, credits).filter((p) => !p.subject.startsWith('text:')),
    ...fields.changes,
  ];
  return { html: fields.html, problems, changed: fields.html !== html, sites, credits };
}

/** One line of counts for the page as written. */
export function pageTally(html) {
  const sites = cardCreditSites(html);
  const ext = sites.filter((s) => s.kind === 'extensions').length;
  const skills = sites.filter((s) => s.kind === 'skills').length;
  const cur = curatorSites(html);
  const shelves = Object.values(cur.shelves).reduce((n, s) => n + s.length, 0);
  return `${ext + skills} card lines (${ext} extension, ${skills} skill), ${shelves} shelf curator lines, ` +
    `${cur.footer.length} footer curator line, ${skills} skill cards' version and licence`;
}

function main(argv) {
  const args = argv.slice(2);
  const unknown = args.filter((a) => a !== '--check');
  if (unknown.length) {
    console.error(`write-credits: unknown argument ${unknown.join(' ')} (the only flag is --check)`);
    return 1;
  }
  const check = args.includes('--check');
  let html;
  let result;
  try {
    html = readFileSync(PAGE, 'utf8');
    result = regeneratePage(html, loadCatalogs());
  } catch (err) {
    console.error(`write-credits: ${err.message}`);
    return 1;
  }
  if (result.problems.length) {
    for (const p of result.problems) console.error(`write-credits: [${p.subject}] ${p.message}`);
    console.error(`write-credits: ${result.problems.length} problem(s); landing/baam.html was not written`);
    return 1;
  }
  if (check) {
    if (!result.changed) {
      console.log(`write-credits: landing/baam.html is current (${pageTally(html)})`);
      return 0;
    }
    const shown = result.sites.slice(0, CHECK_LIST_MAX);
    for (const s of shown) console.error(`write-credits: [${s.subject}] ${s.message}`);
    if (result.sites.length > shown.length) console.error(`write-credits: and ${result.sites.length - shown.length} more`);
    if (result.sites.length === 0) console.error('write-credits: the page differs from what the catalogs give, at no site this script names (whitespace around a credit line?)');
    console.error('write-credits: landing/baam.html is not what the catalogs give; run `node landing/scripts/write-credits.mjs`');
    return 1;
  }
  if (!result.changed) {
    console.log(`write-credits: landing/baam.html is already current (${pageTally(html)})`);
    return 0;
  }
  const tmp = `${PAGE}.tmp-${process.pid}`;
  try {
    writeFileSync(tmp, result.html);
    renameSync(tmp, PAGE);
  } finally {
    rmSync(tmp, { force: true });
  }
  console.log(`write-credits: landing/baam.html written, ${result.sites.length} site(s) changed (${pageTally(result.html)})`);
  return 0;
}

const invoked = (() => {
  try {
    return process.argv[1] && import.meta.url === pathToFileURL(realpathSync(process.argv[1])).href;
  } catch {
    return false;
  }
})();
if (invoked) process.exitCode = main(process.argv);
