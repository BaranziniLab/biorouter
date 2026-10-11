/**
 * Getting a usable `node-pty` out of the Electron main process.
 *
 * `loadNodePty` in `main.ts` used to be one line, `await import('node-pty')`, and
 * trusted whatever came back. node-pty is a CommonJS package, so that line goes
 * through Node's ESM-to-CJS bridge, and the bridge can hand back a module
 * namespace whose NAMES are right and whose VALUES are all `undefined`: the
 * namespace lists `spawn`, `fork` and `open`, and `typeof ns.spawn` is
 * `'undefined'`. The terminal dock then printed
 * `Could not start terminal: pty.spawn is not a function` on every tab, and
 * because the import itself had not thrown, the pipe fallback never ran.
 *
 * Seen in a dev app launched from a git worktree whose `node_modules` is a
 * symlink into the main checkout (round 3 of the redesign review, V1-R3-1). In
 * that process `Module._cache` held two entries for node-pty's `lib/index.js`:
 * the real path, fully loaded with a working `spawn`, and the worktree path,
 * `loaded: false` with empty exports. The ESM namespace had been built from the
 * second. `createRequire(<main.js>)('node-pty')` in the SAME process returned
 * the first, so the plain CommonJS loader is immune. Fresh processes in the same
 * layout (Electron run as Node with that directory as its cwd, and a bare
 * Electron main process over a symlinked `node_modules`) did not reproduce it, so
 * what puts a long-lived process in that state is not pinned down. The cure does
 * not need it to be: do not depend on the bridge.
 *
 * So: ask the CommonJS loader first, keep the import as a second chance, and
 * accept a candidate only when it really has a `spawn` function. Anything else
 * is reported with its shape and the caller falls back to pipes, instead of
 * surfacing a `TypeError` from the first keystroke.
 */
import type * as NodePty from 'node-pty';

export type NodePtyModule = typeof NodePty;

export interface NodePtyLoader {
  /** Names the loader in the log line: `require` or `import`. */
  via: string;
  /** May return the module or a promise of it. A throw or a rejection is reported, not propagated. */
  load: () => unknown;
}

export interface NodePtyLoadResult {
  module: NodePtyModule | null;
  /** The loader that produced `module`, or null when none did. */
  via: string | null;
  /** One line per loader that produced nothing usable, in the order they were tried. */
  problems: string[];
}

function isObjectLike(value: unknown): value is Record<string, unknown> {
  return (typeof value === 'object' || typeof value === 'function') && value !== null;
}

/**
 * The part of a loader's result that can start a shell, or null.
 *
 * `candidate` is either CommonJS `module.exports` or an ESM namespace. A namespace
 * whose names were detected carries `spawn` itself; one whose names were not
 * carries only `default`, which is `module.exports`. A `spawn` that is not a
 * function does not count, which is the whole point: the broken namespace above
 * has the key and not the function.
 */
export function usableNodePty(candidate: unknown): NodePtyModule | null {
  if (!isObjectLike(candidate)) return null;
  if (typeof candidate.spawn === 'function') return candidate as unknown as NodePtyModule;
  const inner = candidate.default;
  if (isObjectLike(inner) && typeof inner.spawn === 'function') {
    return inner as unknown as NodePtyModule;
  }
  return null;
}

/** A one-line description of what a loader returned, for the log of a load that did not work. */
export function describeNodePtyShape(candidate: unknown): string {
  if (!isObjectLike(candidate)) return `got ${candidate === null ? 'null' : typeof candidate}`;
  const inner = candidate.default;
  const names = Object.keys(candidate);
  return (
    `names [${names.join(', ')}], spawn is ${typeof candidate.spawn}, ` +
    `default.spawn is ${isObjectLike(inner) ? typeof inner.spawn : typeof inner}`
  );
}

/**
 * Try each loader in order and return the first usable module.
 *
 * Loaders after the winner are not called. A loader that throws, rejects or
 * returns something unusable is recorded in `problems` and the next one runs; the
 * caller decides how loudly to report them.
 */
export async function loadNodePtyFrom(
  loaders: readonly NodePtyLoader[]
): Promise<NodePtyLoadResult> {
  const problems: string[] = [];
  for (const { via, load } of loaders) {
    try {
      const candidate = await load();
      const module = usableNodePty(candidate);
      if (module) return { module, via, problems };
      problems.push(
        `${via}: loaded, but there is no spawn function (${describeNodePtyShape(candidate)})`
      );
    } catch (error) {
      problems.push(`${via}: ${error instanceof Error ? error.message : String(error)}`);
    }
  }
  return { module: null, via: null, problems };
}
