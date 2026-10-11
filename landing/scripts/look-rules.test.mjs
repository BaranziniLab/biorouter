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
// Fills before lines (owner message 13, R-27): a card, a field, a button or a chip rests on the
// subtle fill with a TRANSPARENT 1px edge, never a drawn outline. What may still draw one: the
// floating surfaces the animations show (a menu, a picker, a dialog keep a hairline and a
// shadow), the schedule's ring marker, and Crew's own recipes, which the app has not converted.
const OUTLINE_OK = /hub-pick|wall-pop|sk-picker|apps-ask|sch-dot|crew-/;
// The composer cards the animations draw are the one box the fill rule does not reach (owner message
// 17, R-32): the page ground with the app's resting hairline, --border-subtle at 60%, which the site
// writes as --line-composer. A card that goes back to the subtle fill with a transparent edge is the
// drift this names. The hairline's colour is a token of its own, so DRAWN_EDGE does not read it as
// an outline.
const COMPOSER_CARDS = {
  'art/hub.css': '.art-hub .hub-card',
  'art/wall.css': '.art-wall .wall-card',
  'art/skills.css': '.art-skills .sk-composer',
};
const DRAWN_EDGE = /^[\d.]+px (solid|dashed|dotted) (var\(--(line|line-2|line-input|coral|coral-bar|ink|navy|danger)\)|#[0-9a-f]{3,8}|rgba?\([^)]*\))$/i;

/** Every violation in a { file: css } map, as strings naming file, selector and why. */
export function violations(files) {
  const found = [];
  for (const [file, css] of Object.entries(files)) {
    const art = file.startsWith('art/');
    // A later rule may restate only the card's padding (a container query), so each declaration that
    // IS written is checked, and the file must write both somewhere.
    const card = COMPOSER_CARDS[file];
    const seen = new Set();
    for (const { selector, body } of rules(css)) {
      if (card && selector === card) {
        for (const [prop, value] of decls(body)) {
          if (prop === 'background') {
            seen.add(prop);
            if (value !== 'var(--paper)') found.push(`${file}: ${selector} { background: ${value} }: the composer card is the page ground (R-32)`);
          }
          if (prop === 'border') {
            seen.add(prop);
            if (value !== '1px solid var(--line-composer)') {
              found.push(`${file}: ${selector} { border: ${value} }: the composer card rests on the app's hairline, --line-composer (R-32)`);
            }
          }
        }
      }
      // A box that paints its own outline: a full border in a visible colour together with a radius.
      const all = decls(body);
      const edge = all.find(([p, v]) => p === 'border' && DRAWN_EDGE.test(v));
      if (edge && all.some(([p]) => p === 'border-radius') && !OUTLINE_OK.test(selector)) {
        found.push(`${file}: ${selector} { border: ${edge[1]} }: boxes rest on the fill with a transparent edge`);
      }
      for (const [prop, value] of all) {
        if (prop === 'box-shadow' && /inset 0(px)? 0(px)? 0(px)? 1px/.test(value) && !OUTLINE_OK.test(selector)) {
          found.push(`${file}: ${selector} { box-shadow: ${value} }: a ring is an outline`);
        }
      }
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
    if (card) {
      for (const prop of ['background', 'border']) {
        if (!seen.has(prop)) found.push(`${file}: ${card} writes no ${prop}: the composer card is the page ground with the app's hairline (R-32)`);
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
  'an outlined card': ['site.css', (s) => s + '\n.card { border: 1px solid var(--line); border-radius: var(--r-container); background: var(--paper); }'],
  'an outlined button': ['baam.css', (s) => s + '\n.brxt-chip { border: 1px solid var(--line-2); border-radius: var(--r-element); }'],
  'a dashed empty box': ['art/apps.css', (s) => s + '\n[data-art="apps"] .apps-empty { border: 1px dashed var(--line-2); border-radius: var(--r-element); }'],
  'a ring drawn as a shadow': ['art/skills.css', (s) => s + '\n.art-skills .sk-sel { box-shadow: inset 0 0 0 1px var(--line); }'],
  'the hub composer card back on the fill': ['art/hub.css', (s) => s.replace('background: var(--paper);\n  border: 1px solid var(--line-composer)', 'background: var(--fill);\n  border: 1px solid transparent')],
  'the wall composer card back on the fill': ['art/wall.css', (s) => s.replace('background: var(--paper); border: 1px solid var(--line-composer)', 'background: var(--fill); border: 1px solid transparent')],
  'the skills composer card with a harder line': ['art/skills.css', (s) => s.replace('border: 1px solid var(--line-composer)', 'border: 1px solid var(--line)')],
  'a drawn edge on the art frame': ['site.css', (s) => s + '\n.art { border: 1px solid var(--line); border-radius: var(--r-surface); }'],
};
for (const [name, [file, edit]] of Object.entries(mutants)) {
  test(`mutant caught: ${name}`, () => {
    const files = real();
    files[file] = edit(files[file]);
    assert.ok(violations(files).length > 0, `${name} went unnoticed`);
  });
}
