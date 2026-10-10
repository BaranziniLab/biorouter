import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
import { describe, expect, it } from 'vitest';

/**
 * Find in chat is an OVERLAY (spec 3.16), and these are the stylesheet's half of
 * that claim.
 *
 * It used to be a sticky row in the transcript's scroll content that revealed
 * with a `max-height` transition under `overflow: hidden`, so opening it pushed
 * the conversation down while it animated, and the ceiling was also a CLIP:
 * when the bar said "type at least 2 characters" it was two rows, and the
 * one-row ceiling cut that sentence through its middle (measured in the app on
 * 2026-09-12). An absolutely positioned overlay pushes nothing and has no
 * ceiling, so the note row fits by construction, and these tests keep it that
 * way.
 *
 * ⚠ **Asserted at the source, and it has to be.** jsdom has no layout engine and
 * never loads this file, so a component test that reads a position or a height
 * passes whether the rule exists or not.
 */
/** The stylesheet with its comments blanked: the history in them names what is gone. */
const CSS = readFileSync(join(__dirname, 'search.css'), 'utf8').replace(/\/\*[\s\S]*?\*\//g, '');

/** The body of the first rule whose selector is exactly `selector`. */
function ruleBody(selector: string): string {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const match = CSS.match(new RegExp(`(?:^|\\n)${escaped}\\s*\\{([^}]*)\\}`));
  if (!match) throw new Error(`search.css has no rule for ${selector}`);
  return match[1];
}

describe('find in chat floats over the transcript', () => {
  it('is positioned absolutely, top right, so opening it moves nothing', () => {
    const overlay = ruleBody('.br-find-overlay');
    expect(overlay).toMatch(/position:\s*absolute/);
    expect(overlay).toMatch(/top:\s*8px/);
    expect(overlay).toMatch(/right:\s*16px/);
    expect(overlay).not.toMatch(/sticky/);
  });

  it('has no reveal ceiling left to clip the two-row note', () => {
    // The old reveal and its two ceilings are gone for good.
    expect(CSS).not.toMatch(/max-height/);
    expect(CSS).not.toMatch(/--search-bar-height/);
    expect(CSS).not.toMatch(/\.search-bar-(enter|exit)/);
    expect(ruleBody('.br-find-overlay')).not.toMatch(/overflow:\s*hidden/);
  });

  it('opens with an 8px drop and closes with a 4px rise, on the motion tokens', () => {
    expect(ruleBody(".br-find-overlay[data-state='open']")).toMatch(
      /animation:\s*br-find-in var\(--dur-fast-max\) var\(--ease-out\)/
    );
    expect(ruleBody(".br-find-overlay[data-state='closed']")).toMatch(
      /animation:\s*br-find-out var\(--dur-fast\) var\(--ease-out\) forwards/
    );
    const enter = CSS.match(/@keyframes br-find-in\s*\{([\s\S]*?)\n\}/);
    const exit = CSS.match(/@keyframes br-find-out\s*\{([\s\S]*?)\n\}/);
    expect(enter?.[1]).toMatch(/translateY\(-8px\)/);
    expect(exit?.[1]).toMatch(/translateY\(-4px\)/);
    // Only opacity and transform move.
    for (const frames of [enter?.[1] ?? '', exit?.[1] ?? '']) {
      const properties = [...frames.matchAll(/([a-z-]+):/g)].map((m) => m[1]);
      expect(new Set(properties)).toEqual(new Set(['opacity', 'transform']));
    }
  });

  it('rests to a fade under reduced motion', () => {
    const block = CSS.match(/@media \(prefers-reduced-motion: reduce\)\s*\{([\s\S]*?)\n\}/);
    expect(block, 'no reduced-motion block in search.css').not.toBeNull();
    expect(block![1]).toMatch(/\[data-state='open'\]\s*\{\s*animation-name:\s*br-fade-in/);
  });
});

/**
 * `SearchView` is the transcript's find and nothing else (spec 3.16): the list
 * views filter with the band's `FilterInput`. So only the live chat and a saved
 * transcript import it.
 *
 * `PENDING` names a view whose owner has not moved it to `FilterInput` yet. It
 * may only shrink: an entry whose file no longer imports `SearchView` fails the
 * second test, so the owner deletes its line in the same commit.
 */
const ALLOWED = ['components/BaseChat.tsx', 'components/sessions/SessionHistoryView.tsx'];
const PENDING: Record<string, string> = {
  // WS-VIEWS-A: Workflows moves to the band FilterInput (spec 3.10).
  'components/workflows/WorkflowsView.tsx': 'WS-VIEWS-A',
  // WS-EXTENSIONS: Extensions moves to the band FilterInput (spec 3.11).
  'components/extensions/ExtensionsView.tsx': 'WS-EXTENSIONS',
};

const SRC = join(__dirname, '..');

function sourceFiles(dir: string): string[] {
  const out: string[] = [];
  for (const name of readdirSync(dir)) {
    if (name === 'node_modules' || name.startsWith('.')) continue;
    const path = join(dir, name);
    if (statSync(path).isDirectory()) out.push(...sourceFiles(path));
    else if (/\.(ts|tsx)$/.test(name) && !/\.test\.(ts|tsx)$/.test(name)) out.push(path);
  }
  return out;
}

function importsSearchView(path: string): boolean {
  return /from\s+['"][./]*(?:conversation\/)?SearchView['"]/.test(readFileSync(path, 'utf8'));
}

describe('SearchView is the transcript find only', () => {
  const importers = sourceFiles(SRC)
    .filter((path) => !path.endsWith(join('conversation', 'SearchView.tsx')))
    .filter(importsSearchView)
    .map((path) => relative(SRC, path).split('\\').join('/'))
    .sort();

  it('is imported only by the live chat and a saved transcript (plus the pending views)', () => {
    const unexpected = importers.filter((path) => !ALLOWED.includes(path) && !(path in PENDING));
    expect(unexpected).toEqual([]);
    for (const path of ALLOWED) expect(importers).toContain(path);
  });

  it('carries no pending entry that has already moved', () => {
    const moved = Object.keys(PENDING).filter((path) => !importers.includes(path));
    expect(moved, 'delete these PENDING lines: the view no longer imports SearchView').toEqual([]);
  });
});
