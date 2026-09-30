import { readFileSync, writeFileSync, renameSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const repo = new URL('../../', import.meta.url);
const source = new URL('docs/website/', repo);
const output = new URL('landing/docs.html', repo);
const order = ['getstarted', 'app', 'cli', 'providers', 'privacy', 'vaulting', 'extensions', 'workflows', 'knowledge', 'apps', 'crew', 'agentloop', 'config', 'architecture'];
const content = readFileSync(new URL('landing/assets/landing-site-content.md', repo), 'utf8');
const version = content.match(/\*\*Version:\*\* v(\d+\.\d+\.\d+)/)?.[1];
if (!version) throw new Error('The landing content has no published version fallback.');
const shell = readFileSync(new URL('shell.html', source), 'utf8');
if (shell.split('<!--DOC-PAGES-->').length !== 2) throw new Error('The docs shell needs one page insertion point.');
const pages = order.map(name => {
  const fragment = readFileSync(new URL(`pages/${name}.html`, source), 'utf8').trim();
  if (!fragment.startsWith(`<div class="doc-page" id="doc-${name}">`)) throw new Error(`The ${name} fragment has no matching page root.`);
  return fragment;
});
const html = shell.replace('<!--DOC-PAGES-->', pages.join('\n\n')).replaceAll('{{LATEST_VERSION}}', version);
if (process.argv.includes('--check')) {
  if (readFileSync(output, 'utf8') !== html) throw new Error('Run node landing/scripts/assemble-docs.mjs to update landing/docs.html.');
  console.log('All 14 documentation pages match their tracked sources.');
} else {
  const staged = fileURLToPath(output) + `.tmp-${process.pid}`;
  writeFileSync(staged, html);
  renameSync(staged, output);
  console.log('Assembled all 14 documentation pages.');
}
