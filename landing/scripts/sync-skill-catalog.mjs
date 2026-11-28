#!/usr/bin/env node
// Copies biorouter-skills catalog.json into landing/catalog/skills.json and its
// scripts/lib/catalog-rules.mjs over landing/scripts/lib/catalog-rules.mjs, both
// at one commit, then regenerates what reads them: the credit lines on
// landing/baam.html and the published registry and credits files.
//
//   node landing/scripts/sync-skill-catalog.mjs --skills-repo <checkout> [--commit <sha>]
//   node landing/scripts/sync-skill-catalog.mjs --skills-repo <checkout> [--commit <sha>] --mirror-only
//   node landing/scripts/sync-skill-catalog.mjs ... --landing-dir <dir>
//
// The mirror is catalog.json at one commit, byte for byte, with one line added
// after the opening brace: "source_commit": "<sha>". check-catalog.mjs removes
// that line and compares the rest with the commit it names (--skills-repo, and
// --remote in the deploy workflow), so the mirror cannot drift unnoticed. The
// rules file is copied verbatim from the same commit, and check-catalog.mjs
// compares it the same way. The commit defaults to the checkout's origin/main,
// because a mirror of a commit that was never pushed could not be checked by
// the deploy.
//
// The catalog must be in provenance format 2 ("format": 2); any other format is
// refused before anything is written. Both files are read from the commit before
// either is written.
//
// Then, unless --mirror-only, it runs node landing/scripts/write-credits.mjs
// (the card lines, curator texts and skill card versions and licences on the
// page) and node landing/scripts/build-registry.mjs (registry.json, credits.json
// and their app copies), from the repository root, and exits with the first
// non-zero code. One command takes a new skills commit all the way to the page.
//
// --landing-dir <dir> writes into another copy of landing/ (the tests use a
// temporary one); the two generators then run from that copy's scripts/, with
// its parent as the working directory. The default is this repository's
// landing/. No dependencies.

import { execFileSync, spawnSync } from 'node:child_process';
import { mkdirSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const USAGE =
  'usage: node landing/scripts/sync-skill-catalog.mjs --skills-repo <biorouter-skills checkout> [--commit <sha>] [--mirror-only] [--landing-dir <dir>]';
/** The format catalog.json must be in. */
const FORMAT = 2;
const RULES = 'scripts/lib/catalog-rules.mjs';
/** catalog.json is 1.1 MB in format 2; git's output buffer defaults to 1 MiB (ENOBUFS). */
const GIT_MAX_BUFFER = 64 * 1024 * 1024;
const GENERATORS = ['write-credits.mjs', 'build-registry.mjs'];

const argv = process.argv.slice(2);
const die = (code, message) => {
  console.error(message);
  process.exit(code);
};
const opt = (name) => {
  const i = argv.indexOf(name);
  if (i === -1) return null;
  const value = argv[i + 1];
  if (value === undefined || value.startsWith('--')) die(2, `${name} needs a value\n${USAGE}`);
  return value;
};
const repo = opt('--skills-repo');
if (!repo) die(2, USAGE);
const landing = resolve(opt('--landing-dir') ?? join(dirname(fileURLToPath(import.meta.url)), '..'));
const mirrorOnly = argv.includes('--mirror-only');

const git = (...args) =>
  execFileSync('git', ['-C', resolve(repo), ...args], { encoding: 'utf8', maxBuffer: GIT_MAX_BUFFER, stdio: ['ignore', 'pipe', 'pipe'] });
const show = (commit, path) => {
  try {
    return git('show', `${commit}:${path}`);
  } catch (e) {
    return die(1, `${repo} has no ${path} at ${commit}: ${String(e.stderr || e.message).trim()}`);
  }
};

let commit;
try {
  commit = git('rev-parse', '--verify', `${opt('--commit') ?? 'origin/main'}^{commit}`).trim();
} catch (e) {
  die(1, `${repo}: cannot resolve ${opt('--commit') ?? 'origin/main'}: ${String(e.stderr || e.message).trim()}`);
}
const text = show(commit, 'catalog.json');
const rules = show(commit, RULES);

if (!text.startsWith('{\n')) die(1, 'catalog.json does not start with "{" on its own line; the mirror format needs it');
let catalog;
try {
  catalog = JSON.parse(text);
} catch (e) {
  die(1, `catalog.json at ${commit} is not JSON: ${e.message}`);
}
if (catalog?.format !== FORMAT) {
  die(1, `catalog.json at ${commit} says "format": ${JSON.stringify(catalog?.format)}; this mirror takes "format": ${FORMAT} only, and nothing was written`);
}
const out = `{\n  "source_commit": "${commit}",\n${text.slice(2)}`;
JSON.parse(out);

// A fresh --landing-dir has neither folder yet; this repository's landing/ has both.
mkdirSync(join(landing, 'catalog'), { recursive: true });
mkdirSync(join(landing, 'scripts', 'lib'), { recursive: true });
writeFileSync(join(landing, 'catalog', 'skills.json'), out);
writeFileSync(join(landing, 'scripts', 'lib', 'catalog-rules.mjs'), rules);
console.log(`landing/catalog/skills.json mirrors catalog.json at ${commit}`);
console.log(`landing/scripts/lib/catalog-rules.mjs is ${RULES} at ${commit}`);
if (mirrorOnly) process.exit(0);

const root = resolve(landing, '..');
for (const name of GENERATORS) {
  const script = join(landing, 'scripts', name);
  console.log(`running node ${script}`);
  const r = spawnSync(process.execPath, [script], { cwd: root, stdio: 'inherit' });
  if (r.error) die(1, `could not run ${script}: ${r.error.message}`);
  if (r.status !== 0) die(r.status ?? 1, `${name} failed (${r.status === null ? `signal ${r.signal}` : `exit ${r.status}`}); stopped`);
}
console.log('the mirror, the rules copy, the page and the published files are in step');
