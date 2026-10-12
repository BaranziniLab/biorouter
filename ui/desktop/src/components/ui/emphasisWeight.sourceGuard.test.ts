/**
 * No bare `<strong>` or `<b>` in WS-PRIMITIVES' files (vision V4-R7-6; principle 3).
 *
 * Tailwind's preflight gives `b` and `strong` `font-weight: bolder`, which computes to 700, and the
 * type system draws 400 and 600 only. Arial draws both as bold, so no screenshot shows the defect;
 * only a computed weight does (`modalEmphasisWeight.browser.test.tsx`,
 * `diagnosticsDialog.browser.test.tsx`). Those measure the dialogs that had one. This guard holds
 * the count at zero for the files WS-PRIMITIVES owns, so a new emphasis cannot arrive bare: each
 * one names its weight (`font-semibold`, or `font-normal` for a strong that must not stand out).
 *
 * Scope is the ownership row (spec 3.17): `components/ui/**` except the sidebar's two files,
 * `ModalShell.tsx`, and the five class-only dialogs. Other areas' files are their owners' to guard.
 */
import { readFileSync, readdirSync } from 'node:fs';
import { dirname, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));
const COMPONENTS = resolve(here, '..');

const NOT_OURS = new Set(['sidebar.tsx']);
const CLASS_ONLY = [
  'ModalShell.tsx',
  'DependencySetupModal.tsx',
  'DependencyErrorBanner.tsx',
  'UpdateAvailableModal.tsx',
  'AnnouncementModal.tsx',
  'ErrorBoundary.tsx',
];

function uiSources(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) return uiSources(path);
    if (!entry.name.endsWith('.tsx') || /\.(test|spec)\.tsx$/.test(entry.name)) return [];
    return NOT_OURS.has(entry.name) && dir === here ? [] : [path];
  });
}

/** Every `<strong …>` and `<b …>` opening tag whose class names no weight, with its line. */
export function bareEmphasis(source: string): { line: number; tag: string }[] {
  const found: { line: number; tag: string }[] = [];
  for (const match of source.matchAll(/<(strong|b)(\s[^>]*)?>/g)) {
    const attributes = match[2] ?? '';
    if (/\bfont-(semibold|normal)\b/.test(attributes)) continue;
    found.push({
      line: source.slice(0, match.index).split('\n').length,
      tag: match[0].replace(/\s+/g, ' '),
    });
  }
  return found;
}

describe('Emphasis names its weight in WS-PRIMITIVES files (V4-R7-6)', () => {
  it('the matcher: finds a bare strong and b, and passes the ones that name 600 or 400', () => {
    const sample = [
      '<p>Click <strong>Save</strong> to keep it.</p>',
      '<b>Bold</b>',
      '<strong\n  className="text-text-default">Warning:</strong>',
      '<strong className="font-semibold">Fine</strong>',
      '<b className="font-normal">Quiet</b>',
      '<button>Not emphasis</button>',
      '<br />',
    ].join('\n');
    expect(bareEmphasis(sample).map((f) => f.line)).toEqual([1, 2, 3]);
  });

  it('finds no bare strong or b in components/ui, ModalShell or the class-only dialogs', () => {
    const files = [...uiSources(here), ...CLASS_ONLY.map((name) => join(COMPONENTS, name))];
    expect(files.length).toBeGreaterThan(50);
    const offenders = files.flatMap((file) =>
      bareEmphasis(readFileSync(file, 'utf8')).map(
        ({ line, tag }) => `${relative(COMPONENTS, file)}:${line} ${tag}`
      )
    );
    expect(offenders).toEqual([]);
  });
});
