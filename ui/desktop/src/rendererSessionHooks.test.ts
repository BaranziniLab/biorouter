import { describe, it, expect } from 'vitest';
import { readFileSync, existsSync } from 'node:fs';
import { join } from 'node:path';

/**
 * The app's own hooks reach the app's own windows.
 *
 * WHAT WENT WRONG, AND WHY NOTHING CAUGHT IT. Both Biorouter windows are created
 * with `partition: 'persist:biorouter'`. A `persist:` partition is a different
 * `Session` object, and every hook the app installed — both permission handlers,
 * the CSP response header, the proxy, the `Origin` rewrite — was installed on
 * `session.defaultSession`. So none of them governed the window the user was
 * looking at. The only window left on `defaultSession` is the drag ghost, which
 * loads a `data:` URL with no preload and no node.
 *
 * It was invisible from the test suite because the two policy tests next door
 * (`frameSrcCsp.test.ts`, `workspaceChannelCsp.test.ts`) pin the CSP **strings**.
 * A string pin cannot tell you which policy is live. It was invisible from the
 * code because two comments asserted the opposite — "Both policies apply to this
 * window and the stricter wins" — and a comment is not a mechanism. It was
 * measured in the running app: a deliberate `frame-src` violation fired exactly
 * ONE `securitypolicyviolation`, whose `originalPolicy` was byte-for-byte the
 * `<meta>` tag. Two enforcing policies produce two events.
 *
 * WHAT THESE ASSERTIONS ARE. They read the real `src/main.ts` as text — the same
 * discipline, and the same comment-stripping, as the two policy tests, for the
 * same reason: the partition and the sessions are discussed at length in the
 * comments around them, and a mention in prose must never be mistaken for a call.
 * What they add that a string pin cannot is the *mechanism*: they fail when a new
 * window names a partition nobody hooked, and when a new hook goes on
 * `defaultSession` alone.
 *
 * They cannot be written as a runtime test. `main.ts` imports `electron` at the
 * top level, there is no Electron in vitest, and the fact under test is which
 * `Session` object a hook was attached to — which jsdom has no concept of.
 */

function resolveFromPackage(relative: string): string {
  const candidates = [
    join(process.cwd(), relative),
    join(process.cwd(), 'ui', 'desktop', relative),
  ];
  const found = candidates.find((p) => existsSync(p));
  if (!found) throw new Error(`could not locate ${relative} from ${process.cwd()}`);
  return found;
}

/** `main.ts` with block and whole-line comments removed. */
function mainProcessCode(): string {
  return readFileSync(resolveFromPackage('src/main.ts'), 'utf-8')
    .replace(/\/\*[\s\S]*?\*\//g, '')
    .replace(/^[ \t]*\/\/.*$/gm, '');
}

/** The body of a top-level `[async] function name(...) { ... }`, by brace matching. */
function functionBody(code: string, name: string): string {
  const signature = new RegExp(`^(?:async )?function ${name}\\b[^{]*\\{`, 'm');
  const match = signature.exec(code);
  if (!match) throw new Error(`main.ts has no top-level function ${name}`);
  let depth = 0;
  for (let i = match.index + match[0].length - 1; i < code.length; i += 1) {
    if (code[i] === '{') depth += 1;
    else if (code[i] === '}') {
      depth -= 1;
      if (depth === 0) return code.slice(match.index, i + 1);
    }
  }
  throw new Error(`unbalanced braces reading ${name}`);
}

/**
 * The single hook this app installs on `defaultSession` and NOT on the
 * renderer's partition, with the reason, so that extending the list is a
 * deliberate edit to this file rather than something that happens quietly.
 *
 * `onBeforeSendHeaders` rewrites every request's `Origin` to the vite dev
 * origin. The renderer does not need it — a packaged `file://` document sends no
 * `Origin` on `fetch` and Electron does not CORS-check a `file://` initiator, and
 * both the packaged (`file://`) and dev (`http://localhost:517x`) origins are
 * already admitted by the daemon's socket gates. Extending it would replace the
 * renderer's real identity with a constant this process invented, at exactly the
 * gates that exist to check that identity.
 */
const DEFAULT_SESSION_ONLY_HOOKS = ['onBeforeSendHeaders'];

describe('renderer session hooks', () => {
  it('declares the renderer partition exactly once', () => {
    const code = mainProcessCode();
    const declarations = [...code.matchAll(/const RENDERER_PARTITION\s*=\s*'([^']+)'/g)];
    expect(declarations).toHaveLength(1);
    expect(declarations[0][1]).toBe('persist:biorouter');
  });

  /**
   * The drift this exists to catch: a window that names a partition literal
   * while the hooks are installed on a different one. That is precisely the
   * shape of the original defect, one level up.
   */
  it('names no partition literal in a window that the hooks are not installed on', () => {
    const code = mainProcessCode();
    const literals = [...code.matchAll(/\bpartition:\s*'([^']*)'/g)].map((m) => m[1]);
    expect(literals).toEqual([]);

    const viaConstant = [...code.matchAll(/\bpartition:\s*RENDERER_PARTITION\b/g)];
    // Non-vacuous: the main window and the launcher. A refactor that drops one
    // of them should fail here rather than pass on an empty scan.
    expect(viaConstant.length).toBeGreaterThanOrEqual(2);
  });

  it('installs the hooks on the renderer partition as well as the default session', () => {
    const body = functionBody(mainProcessCode(), 'appSessions');
    expect(body).toMatch(/session\.defaultSession\b/);
    expect(body).toMatch(/session\.fromPartition\(RENDERER_PARTITION\)/);
  });

  /**
   * The hooks are installed before this function's first `await`, because
   * `open-url` and the `.brxt` handlers create windows off their own
   * `app.whenReady()` continuations. Anything after an `await` here can be
   * overtaken by one of them, and a window created first never sees the policy.
   */
  it('installs them before appMain yields', () => {
    const body = functionBody(mainProcessCode(), 'appMain');
    const install = body.indexOf('installSessionHooks(');
    const firstAwait = body.indexOf('await ');
    expect(install).toBeGreaterThan(-1);
    expect(firstAwait).toBeGreaterThan(-1);
    expect(install).toBeLessThan(firstAwait);
  });

  /**
   * Every hook on `defaultSession` has a twin on the renderer partition —
   * i.e. it lives in `installSessionHooks`, which every session in
   * `appSessions()` receives — unless it is one of the documented exceptions.
   */
  it('leaves no default-session hook without a partition twin', () => {
    const code = mainProcessCode();
    const installed = functionBody(code, 'installSessionHooks');

    const onDefault = [
      ...code.matchAll(/session\.defaultSession\.(?:webRequest\.)?([A-Za-z]+)\s*\(/g),
    ].map((m) => m[1]);

    // Non-vacuous: a regex that matched nothing would pass while saying nothing.
    expect(onDefault.length).toBeGreaterThan(0);

    const untwinned = [...new Set(onDefault)].filter(
      (hook) => !DEFAULT_SESSION_ONLY_HOOKS.includes(hook) && !installed.includes(`${hook}(`)
    );
    expect(untwinned).toEqual([]);
  });

  it('installs every documented default-session-only hook in its own named function', () => {
    const body = functionBody(mainProcessCode(), 'installDefaultSessionOnlyHooks');
    for (const hook of DEFAULT_SESSION_ONLY_HOOKS) {
      expect(body).toContain(`${hook}(`);
    }
  });

  /**
   * `app.on('session-created')` would reach EVERY session this process makes,
   * including `persist:biorouter-embedded-browser` and each ephemeral
   * `biorouter-managed-app-<uuid>`. Both install their own, different rules on
   * purpose, and handing the live browser `default-src 'self'` would break every
   * page it exists to show. The app's policy goes on the app's own sessions, by
   * name.
   */
  it('does not blanket every session in the process', () => {
    expect(mainProcessCode()).not.toMatch(/app\.on\(\s*'session-created'/);
  });
});
