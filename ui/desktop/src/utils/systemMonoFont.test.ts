import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import {
  isSystemMonoFontReady,
  loadSystemMonoFont,
  resetSystemMonoFontForTest,
  SYSTEM_MONO_FAMILY,
} from './systemMonoFont';

/**
 * The code face (design.md 9.15): the installed SF Mono registered as "SF Mono", the
 * name Codex's code stack already lists, because Chromium cannot reach it by name.
 */

/** The descriptors a FontFace is built with (DOM's own type is not visible to eslint here). */
type Descriptors = { style?: string; weight?: string };

interface FakeFace {
  family: string;
  source: unknown;
  descriptors: Descriptors | undefined;
}

let made: FakeFace[];
let added: FakeFace[];

beforeEach(() => {
  resetSystemMonoFontForTest();
  made = [];
  added = [];
  vi.stubGlobal(
    'FontFace',
    class {
      family: string;
      source: unknown;
      descriptors: Descriptors | undefined;
      constructor(family: string, source: unknown, descriptors?: Descriptors) {
        this.family = family;
        this.source = source;
        this.descriptors = descriptors;
        made.push(this);
      }
      load() {
        return Promise.resolve(this);
      }
    }
  );
  Object.defineProperty(document, 'fonts', {
    configurable: true,
    value: { add: (face: FakeFace) => added.push(face) },
  });
});

afterEach(() => {
  vi.unstubAllGlobals();
  delete (window as { electron?: unknown }).electron;
});

function withElectron(getSystemMonoFont?: () => Promise<unknown>) {
  (window as { electron?: unknown }).electron = getSystemMonoFont ? { getSystemMonoFont } : {};
}

describe('loadSystemMonoFont', () => {
  it('registers the regular and italic files under "SF Mono", over the whole weight range', async () => {
    withElectron(async () => ({ regular: new Uint8Array([1, 2]), italic: new Uint8Array([3]) }));
    expect(await loadSystemMonoFont()).toBe(true);
    expect(SYSTEM_MONO_FAMILY).toBe('SF Mono');
    expect(added.map((f) => [f.family, f.descriptors?.style, f.descriptors?.weight])).toEqual([
      ['SF Mono', 'normal', '100 900'],
      ['SF Mono', 'italic', '100 900'],
    ]);
    expect(isSystemMonoFontReady()).toBe(true);
  });

  it('asks the main process once, however many surfaces wait on it', async () => {
    const get = vi.fn(async () => ({ regular: new Uint8Array([1]), italic: null }));
    withElectron(get);
    await Promise.all([loadSystemMonoFont(), loadSystemMonoFont()]);
    expect(get).toHaveBeenCalledTimes(1);
    expect(added).toHaveLength(1);
  });

  it('leaves the stack to fall through off macOS, in a browser, or when the file is missing', async () => {
    withElectron(async () => null);
    expect(await loadSystemMonoFont()).toBe(false);
    resetSystemMonoFontForTest();
    withElectron();
    expect(await loadSystemMonoFont()).toBe(false);
    expect(made).toEqual([]);
    expect(isSystemMonoFontReady()).toBe(false);
  });
});

describe('the code stack', () => {
  it("is Codex's, verbatim, so the registered face is the one it names", () => {
    const css = readFileSync(join(__dirname, '..', 'styles', 'main.css'), 'utf8');
    const stack = css
      .match(/--font-mono:\s*([^;]+);/)?.[1]
      .replace(/\s+/g, ' ')
      .trim();
    expect(stack).toBe(
      "ui-monospace, SFMono-Regular, 'SF Mono', Menlo, Consolas, 'Liberation Mono', monospace"
    );
  });
});
