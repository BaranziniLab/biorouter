#!/usr/bin/env node --test
// A source guard for the site's look rules (landing/README.md, "Look rules").
//
// The site shares the desktop app's type scale, weights, control heights and
// motion curve. None of that is visible to a link or a registry check, so a new
// stylesheet rule could write 15px, weight 500 or a 48px button and every
// existing gate would stay green. This reads the stylesheets as text, with no
// browser, and fails on the drift the redesign removed (measured: 12 to 19
// rendered sizes per page before it, at most 8 after).
//
// Each rule has a mutant below: an edit to a copy of the real CSS that the rule
// must catch, so the guard cannot quietly become `return []`.
import { readFileSync, readdirSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import assert from 'node:assert/strict';

const landing = join(dirname(fileURLToPath(import.meta.url)), '..');
const PAGE_CSS = ['site.css', 'home.css', 'download.css', 'docs.css', 'baam.css', 'about.css'];
const ART_CSS = readdirSync(join(landing, 'art')).filter((f) => f.endsWith('.css')).map((f) => `art/${f}`);
const read = (f) => readFileSync(join(landing, f), 'utf8');

/** Every rule as { selector, body }, with comments removed and @media flattened. */
function rules(css) {
  const bare = css.replace(/\/\*[\s\S]*?\*\//g, '');
  const out = [];
  for (const m of bare.matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
    out.push({ selector: m[1].trim().replace(/\s+/g, ' '), body: m[2] });
  }
  return out;
}
const decls = (body) =>
  body.split(';').map((d) => d.trim()).filter(Boolean).map((d) => {
    const i = d.indexOf(':');
    return [d.slice(0, i).trim(), d.slice(i + 1).trim()];
  });

const ART_SIZES = new Set(['12px', '13px', '14px']);
// Selectors that may use --faint: the soft second line of a large headline, icons,
// decorative glyphs and disabled rows. Small readable text uses --subtle.
const FAINT_OK = /\.soft\b|row-toggle|baam-search-wrap svg|docs-search svg|hub-prow\.off|wall-barred|crew-hash|crew-chan svg|sk-send|i-pending/;
// Heights the redesign retired from controls. 44px stays for touch targets and avatars.
const RETIRED_HEIGHTS = new Set(['34px', '36px', '42px', '48px']);
const DIAGRAM = /arrow/;
const TOUCH_44 = /menu-btn|avatar/;

/** Every violation in a { file: css } map, as strings naming file, selector and why. */
export function violations(files) {
  const found = [];
  for (const [file, css] of Object.entries(files)) {
    const art = file.startsWith('art/');
    for (const { selector, body } of rules(css)) {
      for (const [prop, value] of decls(body)) {
        const at = `${file}: ${selector} { ${prop}: ${value} }`;
        if (prop === 'font-size') {
          if (art) {
            if (/^[\d.]+px$/.test(value) && !ART_SIZES.has(value)) found.push(`${at}: art text is 12, 13 or 14px`);
          } else if (!/^(var\(--fs-[a-z0-9]+\)|inherit)$/.test(value) && !(file === 'site.css' && selector === ':root')) {
            found.push(`${at}: page text takes a --fs-* token`);
          }
        }
        if (/^--(wall-t-\w+|kn-name|crew-fs-body|wf-ui|wf-code)$/.test(prop) && !ART_SIZES.has(value)) {
          found.push(`${at}: art text is 12, 13 or 14px`);
        }
        if (prop === 'font-weight' && !['400', '700'].includes(value)) {
          // Inter 600 sets the wordmark and nothing else.
          if (!(value === '600' && /--brand-font/.test(body))) found.push(`${at}: weights are 400, and 700 for strong`);
        }
        if (prop === 'letter-spacing' && !(/^var\(--tr-[a-z]+\)$/.test(value) || value === '0')) {
          found.push(`${at}: tracking takes a --tr-* token`);
        }
        if (prop === 'text-transform' && /uppercase/.test(value)) found.push(`${at}: the site has no caps labels`);
        if (/^(transition|animation|transition-timing-function|animation-timing-function)$/.test(prop)) {
          if (/(^|[\s,])ease(-in|-out|-in-out)?(?=[\s,;]|$)/.test(value)) found.push(`${at}: use var(--ease)`);
        }
        if (prop === 'color' && /var\(--faint\)/.test(value) && !FAINT_OK.test(selector)) {
          found.push(`${at}: small text uses --subtle`);
        }
        if (!art && (prop === 'height' || prop === 'min-height') && /^[\d]+px$/.test(value)) {
          if (RETIRED_HEIGHTS.has(value) && !DIAGRAM.test(selector)) found.push(`${at}: controls are 32 or 40px`);
          if (value === '44px' && !TOUCH_44.test(selector)) found.push(`${at}: 44px is for touch targets`);
        }
      }
    }
  }
  return found;
}

const real = () => Object.fromEntries([...PAGE_CSS, ...ART_CSS].map((f) => [f, read(f)]));

test('the site stylesheets keep the look rules', () => {
  assert.deepEqual(violations(real()), []);
});

// Mutants: each edit must produce at least one violation.
const mutants = {
  'a page text size off the scale': ['home.css', (s) => s + '\n.x { font-size: 15px; }'],
  'a page text size in em': ['docs.css', (s) => s + '\n.x { font-size: 0.86em; }'],
  'a weight Arial cannot draw': ['about.css', (s) => s + '\n.x { font-weight: 500; }'],
  'a heading in 600 outside the wordmark': ['site.css', (s) => s + '\n.x { font-weight: 600; }'],
  'tracking written by hand': ['site.css', (s) => s + '\n.x { letter-spacing: -0.05em; }'],
  'a caps label': ['baam.css', (s) => s + '\n.x { text-transform: uppercase; }'],
  'a bare ease on a hover': ['download.css', (s) => s + '\n.x { transition: color 125ms ease; }'],
  'a bare ease-in-out on a pulse': ['art/hub.css', (s) => s + '\n.x { animation: p 2.4s ease-in-out infinite; }'],
  'faint on a paragraph': ['about.css', (s) => s + '\n.news-row p { color: var(--faint); }'],
  'a 48px button': ['baam.css', (s) => s + '\n.x { height: 48px; }'],
  'a 36px button': ['download.css', (s) => s + '\n.btn-get { height: 36px; }'],
  'a 44px button that is no touch target': ['home.css', (s) => s + '\n.btn { height: 44px; }'],
  'art text at 11.5px': ['art/wall.css', (s) => s + '\n.x { font-size: 11.5px; }'],
  'an art size variable off the scale': ['art/wall.css', (s) => s + '\n.art-wall { --wall-t-msg: 15px; }'],
};
for (const [name, [file, edit]] of Object.entries(mutants)) {
  test(`mutant caught: ${name}`, () => {
    const files = real();
    files[file] = edit(files[file]);
    assert.ok(violations(files).length > 0, `${name} went unnoticed`);
  });
}
