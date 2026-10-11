import { readdirSync, readFileSync, statSync } from 'node:fs';
import { dirname, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

/**
 * Fills before lines in the Workflows, Scheduler and Built apps surfaces (owner message 13,
 * R-27, design.md 9.11), held at the source. The paint is measured in Chromium by
 * `viewsFill.browser.test.tsx`; this file is the cheap half, the part that fails the moment
 * someone writes an outline back or forgets to opt a new list out of its hairlines.
 *
 * A guard that reads source can only say what is written, so each rule below is paired with the
 * case that proves it sees a violation (the fixtures at the end).
 */
const here = dirname(fileURLToPath(import.meta.url));
const COMPONENTS = resolve(here, '..');
const MAIN_CSS = readFileSync(resolve(COMPONENTS, '../styles/main.css'), 'utf8');

const OWNED_DIRS = ['Layout', 'workflows', 'schedule', 'applications', 'parameter'];
const OWNED_FILES = ['ParameterInputModal.tsx', 'NotFoundView.tsx'];
/** `workflows/WorkflowActivities.tsx` belongs to the transcript (WS-TRANSCRIPT). */
const NOT_OURS = new Set(['workflows/WorkflowActivities.tsx']);

function walk(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const full = join(dir, name);
    return statSync(full).isDirectory() ? walk(full) : [full];
  });
}

const sources = [
  ...OWNED_DIRS.flatMap((dir) => walk(resolve(COMPONENTS, dir))),
  ...OWNED_FILES.map((file) => resolve(COMPONENTS, file)),
]
  .filter((file) => file.endsWith('.tsx') && !/\.(browser\.)?test\.tsx$/.test(file))
  .filter((file) => !file.includes('__tests__'))
  .map((file) => ({ name: relative(COMPONENTS, file), text: readFileSync(file, 'utf8') }))
  .filter((file) => !NOT_OURS.has(file.name));

/** A line comment or a block comment's text is prose about the old look, not the look. */
const code = (text: string) => text.replace(/\/\*[\s\S]*?\*\//g, '').replace(/^\s*\/\/.*$/gm, '');

/** A resting outline: a 1px line drawn round a box, or a rule between rows. */
const RESTING_OUTLINE =
  /\b(?:border(?:-[trblxy])?-border-(?:subtle|default|strong|input|emphasized)|divide-[xy]\b|ring-1\b)/;
/** The muted ground IS the fill in both modes, so a box or chip on a filled card vanishes on it. */
const MUTED_ON_FILL = /\bbg-background-(?:muted|medium)\b/;

describe('Views-A under the fill rule: source guards', () => {
  it('reads the files it claims to guard', () => {
    const names = sources.map((s) => s.name);
    expect(names).toEqual(
      expect.arrayContaining([
        'Layout/PageHeader.tsx',
        'workflows/WorkflowsView.tsx',
        'schedule/SchedulesView.tsx',
        'schedule/ScheduleDetailView.tsx',
        'applications/ApplicationsView.tsx',
        'parameter/ParameterInput.tsx',
        'ParameterInputModal.tsx',
      ])
    );
    expect(names).not.toContain('workflows/WorkflowActivities.tsx');
  });

  it('writes no resting outline: no 1px border colour, no divide, no ring at rest', () => {
    const offenders = sources.filter((s) => RESTING_OUTLINE.test(code(s.text))).map((s) => s.name);
    expect(
      offenders,
      'a fill replaces the outline (R-27); keep an edge only for danger or focus'
    ).toEqual([]);
  });

  it('puts no muted or medium ground on a box (it equals the fill, or the old secondary)', () => {
    const offenders = sources.filter((s) => MUTED_ON_FILL.test(code(s.text))).map((s) => s.name);
    expect(offenders).toEqual([]);
  });

  it('opts every list shell out of the hairline between rows', () => {
    const missing: string[] = [];
    for (const { name, text } of sources) {
      for (const match of code(text).matchAll(
        /className="([^"]*\bbiorouter-list-shell\b[^"]*)"/g
      )) {
        if (!/\bbr-rows-flat\b/.test(match[1])) missing.push(`${name}: ${match[1]}`);
      }
    }
    expect(missing).toEqual([]);
    // And the lists are really there to check.
    const shells = sources.reduce(
      (n, s) => n + (code(s.text).match(/\bbiorouter-list-shell\b/g)?.length ?? 0),
      0
    );
    expect(shells).toBeGreaterThanOrEqual(8);
  });

  it('keeps the band’s ground rule and the flat rows’ rule in main.css', () => {
    const band = MAIN_CSS.match(/\.biorouter-page-header\[data-band\] \{[^}]*\}/)?.[0] ?? '';
    expect(band).toMatch(/--br-control-ground:\s*var\(--background-fill-strong\);/);
    expect(MAIN_CSS).toMatch(
      /\.br-rows-flat :is\(\.biorouter-list-row, \.biorouter-settings-row\)::after \{\s*content: none;\s*\}/
    );
  });

  describe('the guard sees what it claims to', () => {
    it('flags each kind of resting outline and a muted box, and not a danger or focus edge', () => {
      for (const bad of [
        'border border-border-subtle',
        'border-t border-border-default',
        'divide-y',
        'ring-1 ring-border-subtle',
      ]) {
        expect(RESTING_OUTLINE.test(bad), bad).toBe(true);
      }
      for (const fine of [
        'border border-transparent',
        'border-border-danger',
        'focus-visible:ring-2 focus-visible:ring-border-focus',
      ]) {
        expect(RESTING_OUTLINE.test(fine), fine).toBe(false);
      }
      expect(MUTED_ON_FILL.test('rounded-inner bg-background-muted')).toBe(true);
      expect(MUTED_ON_FILL.test('bg-background-fill-strong')).toBe(false);
    });

    it('ignores prose in comments and sees a list shell that did not opt out', () => {
      expect(RESTING_OUTLINE.test(code('/* was border-border-subtle */\nconst a = 1;'))).toBe(
        false
      );
      expect(RESTING_OUTLINE.test(code('// border-border-subtle\nconst a = 1;'))).toBe(false);
      const shell = code('<div className="biorouter-list-shell">');
      const match = shell.match(/className="([^"]*\bbiorouter-list-shell\b[^"]*)"/);
      expect(match && /\bbr-rows-flat\b/.test(match[1])).toBe(false);
    });
  });
});
