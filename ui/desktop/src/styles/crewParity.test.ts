import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

/**
 * Crew is the reference design (owner message 5; redesign spec principle 11): the app promotes
 * Crew's recipes under `br-` names and never edits Crew to match. "Refer to Crew" drifts the day
 * either side changes, so this guard (spec 3.2, crew-reference §4.2) reads both stylesheets and
 * asserts that every declaration a Crew original makes is made, with the same value, by its
 * promoted copy. The only exceptions are the values the spec changes on purpose, each listed
 * below with its reason; a copy's extra declarations are listed too, so a new one is a decision
 * rather than drift.
 *
 * The CSS reader is the Crew guard's (`crew/crewCss.sourceGuard.test.ts`), copied rather than
 * imported because a test file exports nothing.
 */

const SRC = join(__dirname, '..');
const read = (path: string) => readFileSync(join(SRC, path), 'utf8');

interface Declaration {
  property: string;
  value: string;
}

interface StyleRule {
  selector: string;
  declarations: Declaration[];
  atRules: string[];
}

function parseDeclarations(body: string): Declaration[] {
  return body
    .split(';')
    .map((part) => part.trim())
    .filter((part) => part.includes(':'))
    .map((part) => {
      const colon = part.indexOf(':');
      return {
        property: part.slice(0, colon).trim().toLowerCase(),
        value: part
          .slice(colon + 1)
          .trim()
          .replace(/\s+/g, ' '),
      };
    });
}

function parseCss(source: string): StyleRule[] {
  const text = source.replace(/\/\*[\s\S]*?\*\//g, ' ');
  const rules: StyleRule[] = [];
  const stack: { prelude: string; body: string }[] = [];
  let buffer = '';
  let quote: string | null = null;
  for (let index = 0; index < text.length; index += 1) {
    const char = text[index];
    if (quote) {
      buffer += char;
      if (char === '\\') {
        buffer += text[index + 1] ?? '';
        index += 1;
      } else if (char === quote) {
        quote = null;
      }
      continue;
    }
    if (char === '"' || char === "'") {
      quote = char;
      buffer += char;
      continue;
    }
    if (char === '{') {
      const lastSemicolon = buffer.lastIndexOf(';');
      if (stack.length > 0) stack[stack.length - 1].body += buffer.slice(0, lastSemicolon + 1);
      stack.push({ prelude: buffer.slice(lastSemicolon + 1).trim(), body: '' });
      buffer = '';
    } else if (char === '}') {
      const frame = stack.pop();
      if (!frame) throw new Error('unbalanced "}" in CSS');
      frame.body += buffer;
      buffer = '';
      const ancestors = stack.map((open) => open.prelude);
      if (
        !frame.prelude.startsWith('@') &&
        !ancestors.some((prelude) => prelude.startsWith('@keyframes'))
      ) {
        rules.push({
          selector: frame.prelude.replace(/\s+/g, ' '),
          declarations: parseDeclarations(frame.body),
          atRules: ancestors.filter((prelude) => prelude.startsWith('@')),
        });
      }
    } else {
      buffer += char;
    }
  }
  if (stack.length > 0) throw new Error('unbalanced "{" in CSS');
  return rules;
}

/** The declarations of every rule whose selector list contains `selector`, merged in order. */
function declarationsOf(rules: StyleRule[], selector: string, media = ''): Map<string, string> {
  const out = new Map<string, string>();
  const wanted = selector.replace(/\s+/g, ' ');
  for (const rule of rules) {
    const parts = rule.selector.split(',').map((part) => part.trim());
    if (!parts.includes(wanted)) continue;
    const inMedia = rule.atRules.filter((at) => at.startsWith('@media')).join(' ');
    if (inMedia !== media) continue;
    for (const { property, value } of rule.declarations) out.set(property, value);
  }
  expect(out.size, `no declarations for ${selector}${media ? ` in ${media}` : ''}`).toBeGreaterThan(
    0
  );
  return out;
}

/** `padding: 0 8px` and `padding-inline: 8px` say the same thing about a row's sides. */
function normalise(map: Map<string, string>): Map<string, string> {
  const out = new Map(map);
  const padding = out.get('padding');
  if (padding && !out.has('padding-inline')) {
    const [block, inline = block] = padding.split(' ');
    out.set('padding-inline', inline);
    if (block !== '0') out.set('padding-block', block);
    out.delete('padding');
  }
  return out;
}

interface Pair {
  name: string;
  crew: { file: string; selector: string; media?: string };
  app: { file: string; selector: string; media?: string };
  /** Properties whose VALUE the spec changes on purpose, each with its reason. */
  changed?: Record<string, string>;
  /** Properties the copy adds, each with its reason. */
  added?: Record<string, string>;
  /** `crew-` names inside values (keyframes) that the copy spells `br-`. */
  renames?: boolean;
}

const MAIN = 'styles/main.css';
const NAV = 'components/BioRouterSidebar/sidebar.css';
const CREW_SIDEBAR = 'components/crew/sidebar/crew-sidebar.css';
const CREW_APP = 'components/crew/crew-app.css';
const CREW_DIALOGS = 'components/crew/dialogs/dialogs.css';
const CREW_PANE = 'components/crew/pane/pane.css';

const DENSITY =
  'Owner decision 6.1: the app sidebar is 28px rows at 13px; Crew keeps 32px at 14px.';

const PAIRS: Pair[] = [
  {
    name: 'the nav row',
    crew: { file: CREW_SIDEBAR, selector: '.crew-sidebar-row' },
    app: { file: NAV, selector: '.br-nav-row' },
    changed: {
      height: DENSITY,
      'font-size': DENSITY,
      'line-height': DENSITY,
      transition:
        'The app row also turns its ink to default on hover (spec 3.4), so colour eases with the ground.',
    },
    added: {
      overflow: 'A 28px row clips its label rather than growing (spec 3.4).',
      'font-weight': 'States the 400 a nav row keeps; Arial draws 500 as 400 anyway (spec 0).',
      'background-color': 'A resting ground the hover and the current row replace.',
    },
  },
  {
    name: 'the group header',
    crew: { file: CREW_SIDEBAR, selector: '.crew-sidebar-team-toggle' },
    app: { file: NAV, selector: '.br-sidebar-folder-toggle' },
    changed: {
      height: 'The folder header fills its 28px row (owner decision 6.1).',
      'font-size': DENSITY,
      'line-height': DENSITY,
      'font-weight':
        'Left at 400: Arial draws 500 as 400 (spec 0), and a folder header is a 13px row like the chats under it.',
    },
    added: {},
  },
  {
    name: 'the row-action reveal',
    crew: { file: CREW_PANE, selector: '.crew-member-actions' },
    app: { file: MAIN, selector: '.br-row-actions' },
    added: {
      display: 'The shared cluster lays its icon buttons out itself.',
      'align-items': 'The shared cluster lays its icon buttons out itself.',
      gap: 'The shared cluster lays its icon buttons out itself.',
    },
  },
  {
    name: 'the row-action reveal without a hover pointer',
    crew: { file: CREW_PANE, selector: '.crew-member-actions', media: '@media (hover: none)' },
    app: { file: MAIN, selector: '.br-row-actions', media: '@media (hover: none)' },
    changed: {
      opacity: 'The same 1, made !important so no host rule can hide it on touch.',
    },
  },
  {
    name: 'the straight hairline',
    crew: { file: CREW_DIALOGS, selector: '.crew-settings-panel .crew-settings-row::after' },
    app: { file: MAIN, selector: '.biorouter-list-row::after' },
  },
  {
    // The declaration is Crew's; the SELECTOR differs on purpose. Crew's row keeps
    // `:focus-within` (Crew is never edited to match), the app's keys the focus half on
    // `:has(:focus-visible)` so a mouse click on a switch leaves no firmed line (spec 2.5).
    name: 'the straight hairline under focus',
    crew: {
      file: CREW_DIALOGS,
      selector: '.crew-settings-panel .crew-settings-row:focus-within::after',
    },
    app: { file: MAIN, selector: '.biorouter-settings-row:has(:focus-visible)::after' },
  },
  {
    name: 'the straight hairline on the last row',
    crew: {
      file: CREW_DIALOGS,
      selector: '.crew-settings-panel .crew-settings-row:last-child::after',
    },
    app: { file: MAIN, selector: '.biorouter-settings-row:last-child::after' },
  },
  {
    name: 'the cross-fade stack',
    crew: { file: CREW_APP, selector: '.crew-crossfade' },
    app: { file: MAIN, selector: '.br-crossfade' },
  },
  {
    name: 'the cross-fade cell',
    crew: { file: CREW_APP, selector: '.crew-crossfade > .crew-crossfade-item' },
    app: { file: MAIN, selector: '.br-crossfade > .br-crossfade-item' },
  },
  {
    name: 'the cross-fade entrance',
    crew: { file: CREW_APP, selector: ".crew-crossfade-item[data-state='open']" },
    app: { file: MAIN, selector: ".br-crossfade-item[data-state='open']" },
    renames: true,
  },
  {
    name: 'the cross-fade exit',
    crew: { file: CREW_APP, selector: ".crew-crossfade-item[data-state='closed']" },
    app: { file: MAIN, selector: ".br-crossfade-item[data-state='closed']" },
    renames: true,
  },
  {
    name: 'the highlight wash',
    crew: { file: CREW_APP, selector: '.crew-highlight' },
    app: { file: MAIN, selector: '.br-highlight' },
    renames: true,
  },
];

const parsed = new Map<string, StyleRule[]>();
const rulesOf = (file: string) => {
  if (!parsed.has(file)) parsed.set(file, parseCss(read(file)));
  return parsed.get(file)!;
};

/** The body of `@keyframes name`, whitespace folded. */
function keyframes(file: string, name: string): string {
  const source = read(file).replace(/\/\*[\s\S]*?\*\//g, ' ');
  const start = source.indexOf(`@keyframes ${name} {`);
  expect(start, `no @keyframes ${name} in ${file}`).toBeGreaterThanOrEqual(0);
  let depth = 0;
  let index = source.indexOf('{', start);
  const open = index;
  for (; index < source.length; index += 1) {
    if (source[index] === '{') depth += 1;
    else if (source[index] === '}') {
      depth -= 1;
      if (depth === 0) break;
    }
  }
  return source.slice(open, index + 1).replace(/\s+/g, ' ');
}

describe('promoted recipes keep their Crew originals (spec 3.2, crew-reference §4.2)', () => {
  it.each(PAIRS.map((pair) => [pair.name, pair] as const))('%s', (_name, pair) => {
    const crew = normalise(
      declarationsOf(rulesOf(pair.crew.file), pair.crew.selector, pair.crew.media)
    );
    const app = normalise(
      declarationsOf(rulesOf(pair.app.file), pair.app.selector, pair.app.media)
    );
    const rename = (value: string) => (pair.renames ? value.replace(/\bcrew-/g, 'br-') : value);

    const drift: string[] = [];
    for (const [property, value] of crew) {
      if (!app.has(property)) {
        if (!(pair.changed && property in pair.changed)) {
          drift.push(`${property}: missing (Crew: ${value})`);
        }
        continue;
      }
      const ours = app.get(property)!.replace(/\s*!important$/, '');
      if (ours !== rename(value) && !(pair.changed && property in pair.changed)) {
        drift.push(`${property}: ${app.get(property)} (Crew: ${value})`);
      }
    }
    for (const property of app.keys()) {
      if (!crew.has(property) && !(pair.added && property in pair.added)) {
        drift.push(`${property}: not in Crew's recipe`);
      }
    }
    // A listed exception that no longer differs is stale, and stale exceptions hide drift.
    for (const property of Object.keys(pair.changed ?? {})) {
      const ours = app.get(property)?.replace(/\s*!important$/, '');
      if (property === 'opacity' && app.get(property)?.endsWith('!important')) continue;
      if (ours !== undefined && ours === rename(crew.get(property) ?? '')) {
        drift.push(`${property}: listed as changed but now equal to Crew's`);
      }
    }
    expect(drift, `${pair.app.selector} has drifted from ${pair.crew.selector}`).toEqual([]);
  });

  it('keeps the cross-fade, fade-out and highlight keyframes frame for frame', () => {
    for (const [crew, app] of [
      ['crew-crossfade-in', 'br-crossfade-in'],
      ['crew-fade-in', 'br-fade-in'],
      ['crew-fade-out', 'br-fade-out'],
      ['crew-highlight-wash', 'br-highlight-wash'],
      ['crew-highlight-hold', 'br-highlight-hold'],
    ]) {
      expect(keyframes(MAIN, app), app).toBe(keyframes(CREW_APP, crew));
    }
  });

  it('names no crew- class in a promoted copy (principle 11)', () => {
    for (const file of [MAIN, NAV]) {
      const selectors = rulesOf(file).map((rule) => rule.selector);
      expect(
        selectors.filter((selector) => /\.crew-/.test(selector)),
        file
      ).toEqual([]);
    }
  });
});
