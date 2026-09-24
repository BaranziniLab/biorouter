import { existsSync, readFileSync, statSync } from 'node:fs';
import { dirname, join, relative, resolve, sep } from 'node:path';
import { describe, expect, it } from 'vitest';

/**
 * The `/crew` route must never load the legacy layout or its stylesheet.
 *
 * `crew/crew.css` is a global stylesheet: once any module on the page imports it, its rules apply
 * everywhere. It styles `.crew-main`, `.crew-channel`, `.crew-timeline`, `.crew-message-meta` and
 * `.crew-message-body` — names the redesigned layout also uses — with padding, 13–14px type and a
 * hover ground across the whole channel column. So the new route is guarded at the source, by
 * walking every module `CrewApp` can load, rather than by a render test: jsdom would load the
 * stylesheet and apply it without complaint.
 *
 * The walker is exercised on the legacy root below, so the guard cannot pass by failing to look.
 */

const CREW_DIR = resolve(__dirname, '..');
const SRC_DIR = resolve(CREW_DIR, '../..');
const LEGACY_DIR = join(CREW_DIR, 'legacy');
const EXTENSIONS = ['.tsx', '.ts', '.css'];

/** Runtime imports only: `import type` and `export type` are erased and load nothing. */
const IMPORT_PATTERN =
  /(?:^|[\s;])(?:import|export)\s+(?!type\s)(?:[^'"`;]*?\sfrom\s*)?['"]([^'"]+)['"]|import\(\s*['"]([^'"]+)['"]\s*\)/g;

function stripComments(source: string): string {
  return source.replace(/\/\*[\s\S]*?\*\//g, ' ').replace(/(^|[^:])\/\/[^\n]*/g, '$1');
}

function resolveImport(from: string, specifier: string): string | null {
  if (!specifier.startsWith('.')) return null;
  const base = resolve(dirname(from), specifier);
  const candidates = [
    base,
    ...EXTENSIONS.map((extension) => `${base}${extension}`),
    ...EXTENSIONS.map((extension) => join(base, `index${extension}`)),
  ];
  return candidates.find((path) => existsSync(path) && statSync(path).isFile()) ?? null;
}

/** Every source file reachable from `entry` through relative runtime imports. */
function reachable(entry: string): Set<string> {
  const seen = new Set<string>();
  const queue = [entry];
  while (queue.length > 0) {
    const file = queue.pop() as string;
    if (seen.has(file)) continue;
    seen.add(file);
    if (file.endsWith('.css')) continue;
    const source = stripComments(readFileSync(file, 'utf8'));
    for (const match of source.matchAll(IMPORT_PATTERN)) {
      const target = resolveImport(file, match[1] ?? match[2]);
      if (target && !seen.has(target)) queue.push(target);
    }
  }
  return seen;
}

const rel = (path: string) => relative(SRC_DIR, path).split(sep).join('/');

function legacyReached(entry: string): string[] {
  return [...reachable(entry)]
    .filter(
      (path) =>
        path === join(CREW_DIR, 'crew.css') ||
        path.startsWith(LEGACY_DIR + sep) ||
        path === join(CREW_DIR, 'CrewView.tsx') ||
        path === join(CREW_DIR, 'CrewFiles.tsx') ||
        path === join(CREW_DIR, 'CrewCredentials.tsx')
    )
    .map(rel)
    .sort();
}

describe('the /crew route never loads the legacy layout', () => {
  it('reaches no legacy module and no legacy stylesheet from CrewApp', () => {
    const files = reachable(join(CREW_DIR, 'CrewApp.tsx'));
    // The walk really covered the new layout and its areas.
    expect([...files].map(rel)).toEqual(
      expect.arrayContaining([
        'components/crew/layout/CrewLayout.tsx',
        'components/crew/crew-app.css',
        'components/crew/sidebar/CrewSidebar.tsx',
        'components/crew/timeline/Timeline.tsx',
        'components/crew/composer/Composer.tsx',
      ])
    );
    expect(legacyReached(join(CREW_DIR, 'CrewApp.tsx'))).toEqual([]);
  });

  it('routes /crew to CrewApp, not to the legacy root', () => {
    const app = stripComments(readFileSync(join(SRC_DIR, 'App.tsx'), 'utf8'));
    expect(app).toMatch(/<Route\s+path="crew"\s+element=\{<CrewApp\s*\/>\}\s*\/>/);
    expect(app).not.toMatch(/['"]\.\/components\/crew\/CrewView['"]/);
    expect(app).not.toMatch(/<CrewView\b/);
  });

  it('would notice: the legacy root reaches the legacy stylesheet', () => {
    expect(legacyReached(join(CREW_DIR, 'CrewView.tsx'))).toEqual(
      expect.arrayContaining([
        'components/crew/crew.css',
        'components/crew/legacy/LegacyCrewLayout.tsx',
      ])
    );
  });
});
