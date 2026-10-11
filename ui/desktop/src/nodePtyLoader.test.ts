/**
 * V1-R3-1: every terminal dock tab printed "Could not start terminal: pty.spawn is
 * not a function". `await import('node-pty')` had resolved, to a module namespace
 * that listed `spawn` and held `undefined` for it, so nothing threw and the pipe
 * fallback never ran. These pin what `loadNodePty` now does with each shape a
 * loader can hand back.
 *
 * The broken namespace is built by hand. It cannot be produced from a vitest
 * process (the state came from a long-lived Electron main process, and a fresh
 * one in the same directory loaded node-pty fine), and the native `pty.node` is
 * built for Electron's ABI, so a test that loaded the real package would fail on
 * every machine for an unrelated reason.
 */
import { describe, it, expect, vi } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describeNodePtyShape, loadNodePtyFrom, usableNodePty } from './nodePtyLoader';

const spawn = () => ({ pid: 1 });
const working = { spawn, fork: spawn, open: spawn, native: {} };

// The namespace seen in the failing process: the right names, no values.
const brokenNamespace = {
  __esModule: true,
  createTerminal: undefined,
  default: undefined,
  fork: undefined,
  native: undefined,
  open: undefined,
  spawn: undefined,
};

describe('usableNodePty', () => {
  it('accepts CommonJS exports that have a spawn function', () => {
    expect(usableNodePty(working)).toBe(working);
  });

  it('accepts a namespace that carries spawn itself', () => {
    const namespace = { ...working, default: working };
    expect(usableNodePty(namespace)).toBe(namespace);
  });

  it('takes module.exports from default when the names were not detected', () => {
    expect(usableNodePty({ default: working })).toBe(working);
  });

  it('refuses a namespace that lists spawn and holds nothing for it', () => {
    expect(usableNodePty(brokenNamespace)).toBeNull();
  });

  it('refuses a spawn that is not a function', () => {
    expect(usableNodePty({ spawn: 'spawn' })).toBeNull();
    expect(usableNodePty({ default: { spawn: {} } })).toBeNull();
  });

  it('refuses things that are not modules at all', () => {
    expect(usableNodePty(undefined)).toBeNull();
    expect(usableNodePty(null)).toBeNull();
    expect(usableNodePty('node-pty')).toBeNull();
    expect(usableNodePty(42)).toBeNull();
  });
});

describe('describeNodePtyShape', () => {
  it('says the names are there and the function is not', () => {
    const line = describeNodePtyShape(brokenNamespace);
    expect(line).toContain('spawn is undefined');
    expect(line).toContain('default.spawn is undefined');
    expect(line).toContain('open, spawn]');
  });

  it('says what a non-object was', () => {
    expect(describeNodePtyShape(undefined)).toBe('got undefined');
    expect(describeNodePtyShape(null)).toBe('got null');
  });
});

describe('loadNodePtyFrom', () => {
  it('returns the first loader that works and never runs the rest', async () => {
    const second = vi.fn(() => working);
    const result = await loadNodePtyFrom([
      { via: 'require', load: () => working },
      { via: 'import', load: second },
    ]);
    expect(result.module).toBe(working);
    expect(result.via).toBe('require');
    expect(result.problems).toEqual([]);
    expect(second).not.toHaveBeenCalled();
  });

  it('moves on from a namespace with no spawn function, the V1-R3-1 shape', async () => {
    const result = await loadNodePtyFrom([
      { via: 'import', load: async () => brokenNamespace },
      { via: 'require', load: () => working },
    ]);
    expect(result.module).toBe(working);
    expect(result.via).toBe('require');
    expect(result.problems).toHaveLength(1);
    expect(result.problems[0]).toMatch(/^import: loaded, but there is no spawn function/);
  });

  it('moves on from a loader that throws, and keeps its message', async () => {
    const result = await loadNodePtyFrom([
      {
        via: 'require',
        load: () => {
          throw new Error("Cannot find module 'node-pty'");
        },
      },
      { via: 'import', load: async () => ({ default: working }) },
    ]);
    expect(result.module).toBe(working);
    expect(result.via).toBe('import');
    expect(result.problems).toEqual(["require: Cannot find module 'node-pty'"]);
  });

  it('moves on from a loader that rejects', async () => {
    const result = await loadNodePtyFrom([
      { via: 'import', load: () => Promise.reject(new Error('dlopen failed')) },
      { via: 'require', load: () => working },
    ]);
    expect(result.module).toBe(working);
    expect(result.problems).toEqual(['import: dlopen failed']);
  });

  it('reports null with a line per loader when none works, so the caller can use pipes', async () => {
    const result = await loadNodePtyFrom([
      { via: 'require', load: () => brokenNamespace },
      {
        via: 'import',
        load: () => {
          throw 'not an Error';
        },
      },
    ]);
    expect(result.module).toBeNull();
    expect(result.via).toBeNull();
    expect(result.problems).toHaveLength(2);
    expect(result.problems[0]).toMatch(/^require: /);
    expect(result.problems[1]).toBe('import: not an Error');
  });

  it('reports null for an empty list', async () => {
    expect(await loadNodePtyFrom([])).toEqual({ module: null, via: null, problems: [] });
  });
});

describe('main.ts loadNodePty', () => {
  // jsdom runs no Electron, so the order the loaders are tried in is pinned at
  // the source, where it is written.
  const main = readFileSync(resolve(__dirname, 'main.ts'), 'utf8');
  const start = main.indexOf('async function loadNodePty()');
  const body = main.slice(start, main.indexOf('function disposeTerminalSession'));

  it('asks the CommonJS loader before the ESM import', () => {
    expect(start).toBeGreaterThan(-1);
    const requireAt = body.indexOf("via: 'require'");
    const importAt = body.indexOf("via: 'import'");
    expect(requireAt).toBeGreaterThan(-1);
    expect(importAt).toBeGreaterThan(requireAt);
    expect(body).toContain("createRequire(__filename)('node-pty')");
  });

  it('does not take the bare import as the answer any more', () => {
    expect(body).not.toMatch(/nodePtyModule\s*=\s*await import\(/);
    expect(body).toContain('loadNodePtyFrom(');
  });

  it('still falls back to pipes when no loader worked', () => {
    expect(body).toContain('falling back to process pipes');
    expect(body).toContain('return null;');
  });
});
