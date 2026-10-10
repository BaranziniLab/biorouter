import { readdirSync, readFileSync } from 'node:fs';
import { join, relative, sep } from 'node:path';
import { describe, expect, it } from 'vitest';

/**
 * One control per job, one look per control (spec principle 5 and 2.6): outside `components/ui/`
 * nothing hand-rolls a control the primitives own. This guard reads every component source file
 * outside `ui/` and fails on:
 *   - a native `type="checkbox"` or `type="radio"` input (use `ui/Checkbox`, `ui/CustomRadio`),
 *     except Crew's task boxes in `crew/timeline/MessageBody.tsx`, which render markdown's own
 *     `- [ ]` items;
 *   - `role="switch"` on anything (use `ui/switch`, a Radix `<button role="switch">`);
 *   - an import from `@radix-ui/themes` (the app's primitives are `components/ui/`);
 *   - `title=` on a menu item (help text never goes in `title=`: AppTooltipLayer turns a title
 *     into the item's name, primitives defect 6);
 *   - the retired Switch `variant` (wave 2 deletes the prop).
 *
 * A file still waiting on its owner's sweep is listed in PENDING with the request that carries
 * the diff. The list only shrinks: an entry that no longer offends fails as stale, so it is
 * removed the day the owner lands the change.
 */

const COMPONENTS = join(__dirname, '..');

function walk(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) return walk(path);
    return [path];
  });
}

const rel = (path: string) => relative(COMPONENTS, path).split(sep).join('/');

const SOURCES = walk(COMPONENTS)
  .filter((path) => /\.(ts|tsx)$/.test(path))
  .filter((path) => !/\.(test|spec)\.(ts|tsx)$/.test(path) && !path.endsWith('.d.ts'))
  .filter((path) => !rel(path).startsWith('ui/'));

type Rule = 'native-choice' | 'role-switch' | 'radix-themes' | 'menu-item-title' | 'switch-variant';

const RULES: Record<Rule, (source: string) => number> = {
  'native-choice': (source) => (source.match(/\stype=["'](?:checkbox|radio)["']/g) ?? []).length,
  'role-switch': (source) => (source.match(/\srole=["']switch["']/g) ?? []).length,
  'radix-themes': (source) => (source.match(/from ['"]@radix-ui\/themes['"]/g) ?? []).length,
  'menu-item-title': (source) =>
    Array.from(source.matchAll(/<(?:DropdownMenu|ContextMenu)\w*Item\b([^>]*?)>/gs)).filter(
      (match) => /\stitle=/.test(match[1])
    ).length,
  'switch-variant': (source) =>
    Array.from(source.matchAll(/<Switch\b([^>]*?)\/?>/gs)).filter((match) =>
      /\svariant=/.test(match[1])
    ).length,
};

/** Allowed for good: what the rule cannot replace. */
const ALLOWED: Partial<Record<Rule, Record<string, string>>> = {
  'native-choice': {
    'crew/timeline/MessageBody.tsx':
      'Markdown task-list boxes, read-only, rendered as Crew draws them.',
  },
};

/** Waiting on the owner's sweep, each with the request that carries the diff. Only shrinks. */
const PENDING: Partial<Record<Rule, Record<string, string>>> = {
  'radix-themes': {
    'settings/providers/modal/subcomponents/forms/CustomProviderForm.tsx':
      'WS-SETTINGS-M, requests/WS-SETTINGS-M/WS-PRIMITIVES-1.md',
  },
  'switch-variant': {
    'settings/capabilities/CapabilitiesSection.tsx':
      'WS-SETTINGS-B, requests/WS-SETTINGS-B/WS-PRIMITIVES-1.md',
    'settings/models/subcomponents/LeadWorkerSettings.tsx':
      'WS-SETTINGS-M, requests/WS-SETTINGS-M/WS-PRIMITIVES-1.md',
    'settings/dictation/VoiceDictationToggle.tsx':
      'Frozen (unmounted Settings folder), requests/coordinator/WS-PRIMITIVES-2.md',
    'settings/sessions/SessionSharingSection.tsx':
      'Frozen (unmounted Settings folder), requests/coordinator/WS-PRIMITIVES-2.md',
  },
};

describe('controls come from components/ui (spec 2.6, the controls source guard)', () => {
  it('reads the component sources', () => {
    expect(SOURCES.length).toBeGreaterThan(100);
    expect(SOURCES.some((path) => rel(path) === 'crew/timeline/MessageBody.tsx')).toBe(true);
  });

  it.each(Object.keys(RULES) as Rule[])('%s', (rule) => {
    const offenders = SOURCES.map((path) => rel(path)).filter(
      (path) => RULES[rule](readFileSync(join(COMPONENTS, path), 'utf8')) > 0
    );
    const allowed = { ...(ALLOWED[rule] ?? {}), ...(PENDING[rule] ?? {}) };
    expect(
      offenders.filter((path) => !(path in allowed)),
      `${rule}: use the primitive in components/ui`
    ).toEqual([]);
    expect(
      Object.keys(PENDING[rule] ?? {}).filter((path) => !offenders.includes(path)),
      `${rule}: swept, so remove it from PENDING`
    ).toEqual([]);
  });

  it('catches the shapes it is written for', () => {
    expect(RULES['native-choice'](' <input type="checkbox" />')).toBe(1);
    expect(RULES['native-choice']('querySelector(\'input[type="radio"]:checked\')')).toBe(0);
    expect(RULES['role-switch']('<button role="switch" />')).toBe(1);
    expect(RULES['radix-themes']("import { Checkbox } from '@radix-ui/themes';")).toBe(1);
    expect(RULES['menu-item-title']('<DropdownMenuItem\n  title="Help"\n  onSelect={go}>')).toBe(1);
    expect(RULES['menu-item-title']('<DropdownMenuItem onSelect={go}>')).toBe(0);
    expect(RULES['switch-variant']('<Switch\n  checked\n  variant="mono"\n/>')).toBe(1);
  });
});
