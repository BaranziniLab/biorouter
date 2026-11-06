/**
 * A folder-access approval's folder and files (`ToolCallPreview`, `folderAccess`).
 *
 * Everywhere a folder is only READ, the app writes it in sans from `~` (utils/folderPath.ts).
 * Here the folder is the exact argument of a security decision, so it stays absolute and mono.
 * What changed is how it wraps: `break-all` split a folder's name between any two letters; now a
 * line breaks only after a separator (each name and the separator after it are one atomic piece,
 * `PathSegments`) and `overflow-wrap: anywhere` is the last resort for one over-long name. The
 * line breaks themselves are measured in Chromium (toolCardsGeometry.browser.test.tsx); jsdom lays
 * nothing out.
 */
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { render } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { ToolCallPreview } from './ToolCallPreview';
import type { ToolPreview } from '../api';

const here = dirname(fileURLToPath(import.meta.url));

const FOLDER = '/Users/someone/Desktop/results';
const FILES = ['/Users/someone/Desktop/results/2026/table.csv', 'C:\\Data\\cohort\\notes.txt'];

const preview = {
  kind: 'folderAccess',
  folder: FOLDER,
  access: 'Read-only',
  files: FILES,
  descendants: true,
  onceDescendants: false,
  actionId: 'action-1',
} as ToolPreview;

/** The drawn paths: the folder, then each file. */
const paths = (container: HTMLElement) => [
  ...container.querySelectorAll<HTMLElement>('[data-testid="folder-access-preview"] [data-path]'),
];

let saved: typeof window.appConfig;

beforeEach(() => {
  saved = window.appConfig;
  // This machine's home and its own daemon: everywhere else this folder would read `~/…`.
  Object.defineProperty(window, 'appConfig', {
    configurable: true,
    value: {
      get: (key: string) =>
        ({ BIOROUTER_HOME_DIR: '/Users/someone', BIOROUTER_LOCAL_BACKEND: true })[key],
    },
  });
});

afterEach(() => {
  Object.defineProperty(window, 'appConfig', { configurable: true, value: saved });
});

describe('the folder-access approval card', () => {
  it('writes the folder and every file exactly as the call names them, in mono, never `~`', () => {
    const { container } = render(<ToolCallPreview preview={preview} />);
    const drawn = paths(container);
    expect(drawn.map((node) => node.dataset.path)).toEqual([FOLDER, ...FILES]);
    for (const node of drawn) {
      expect(node.textContent, 'the exact argument, nothing added or written as ~').toBe(
        node.dataset.path
      );
      expect(node.className).toMatch(/\bfont-mono\b/);
      expect(node.className).toMatch(/\bbr-tool-preview-path\b/);
      expect(node).toHaveAttribute('dir', 'ltr');
    }
  });

  it('has no break-all anywhere in it', () => {
    const { container } = render(<ToolCallPreview preview={preview} />);
    const card = container.querySelector('[data-testid="folder-access-preview"]')!;
    for (const node of [card, ...card.querySelectorAll('*')]) {
      expect(node.getAttribute('class') ?? '').not.toMatch(/break-all/);
    }
  });

  it('isolates each name and groups it with the separator after it, the leading one first', () => {
    const { container } = render(<ToolCallPreview preview={preview} />);
    const [folder, , windows] = paths(container);
    const pieces = (path: Element) =>
      [...path.querySelectorAll('.br-path-piece')].map((piece) => piece.textContent);
    expect([...folder.querySelectorAll('bdi')].map((node) => node.textContent)).toEqual([
      'Users',
      'someone',
      'Desktop',
      'results',
    ]);
    // A line breaks only between two pieces, after a separator, and a root never ends a line
    // alone.
    expect(pieces(folder)).toEqual(['/Users/', 'someone/', 'Desktop/', 'results']);
    // A backslash path breaks after its backslashes too, where Chromium would not on its own.
    expect([...windows.querySelectorAll('bdi')].map((node) => node.textContent)).toEqual([
      'C:',
      'Data',
      'cohort',
      'notes.txt',
    ]);
    expect(pieces(windows)).toEqual(['C:\\', 'Data\\', 'cohort\\', 'notes.txt']);
  });

  it('wraps by an authored rule: overflow-wrap anywhere, not break-all', () => {
    const css = readFileSync(resolve(here, 'tool-call.css'), 'utf8');
    const rule = css.match(/\.br-tool-preview-path\s*\{([^}]*)\}/)?.[1] ?? '';
    expect(rule).toMatch(/overflow-wrap:\s*anywhere/);
    expect(rule).not.toMatch(/word-break/);
    // Each piece (a name and its separator) is one atomic box no wider than the line.
    const piece = css.match(/\.br-tool-preview-path \.br-path-piece\s*\{([^}]*)\}/)?.[1] ?? '';
    expect(piece).toMatch(/display:\s*inline-block/);
    expect(piece).toMatch(/max-width:\s*100%/);
    const source = readFileSync(resolve(here, 'ToolCallPreview.tsx'), 'utf8');
    expect(source).not.toMatch(/break-all/);
  });
});
