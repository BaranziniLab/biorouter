#!/usr/bin/env node
// Copies biorouter-skills catalog.json into landing/catalog/skills.json.
//
//   node landing/scripts/sync-skill-catalog.mjs --skills-repo <checkout> [--commit <sha>]
//
// The mirror is catalog.json at one commit, byte for byte, with one line added
// after the opening brace: "source_commit": "<sha>". check-catalog.mjs removes
// that line and compares the rest with the commit it names (--skills-repo, and
// --remote in the deploy workflow), so the mirror cannot drift unnoticed.
// The commit defaults to the checkout's origin/main, because a mirror of a
// commit that was never pushed could not be checked by the deploy.

import { execFileSync } from 'node:child_process';
import { writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { CATALOG_DIR } from './check-catalog.mjs';

const argv = process.argv.slice(2);
const opt = (name) => {
  const i = argv.indexOf(name);
  return i === -1 ? null : argv[i + 1];
};
const repo = opt('--skills-repo');
if (!repo) {
  console.error('usage: node landing/scripts/sync-skill-catalog.mjs --skills-repo <biorouter-skills checkout> [--commit <sha>]');
  process.exit(2);
}
const git = (...args) => execFileSync('git', ['-C', resolve(repo), ...args], { encoding: 'utf8' });
const commit = git('rev-parse', opt('--commit') ?? 'origin/main').trim();
const text = git('show', `${commit}:catalog.json`);
if (!text.startsWith('{\n')) {
  console.error('catalog.json does not start with "{" on its own line; the mirror format needs it');
  process.exit(1);
}
const out = `{\n  "source_commit": "${commit}",\n${text.slice(2)}`;
JSON.parse(out);
writeFileSync(join(CATALOG_DIR, 'skills.json'), out);
console.log(`landing/catalog/skills.json mirrors catalog.json at ${commit}`);
