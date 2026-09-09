/**
 * The sandbox invariant for the Playwright suite, pinned at the source.
 *
 * ⚠ **Why this test exists, measured rather than imagined.** On 2026-09-09 a run
 * of the whole e2e directory wrote an entry into the developer's REAL
 * `~/.local/share/biorouter/projects.json` — because `brxt.spec.ts` called
 * `electron.launch` with no `BIOROUTER_PATH_ROOT`, so the app it started resolved
 * its config, data and state under the developer's own home. Nothing failed; the
 * suite went green while writing outside its box. `helpers/app.ts` now creates
 * the sandbox itself so a call site cannot forget it, but a *comment* saying
 * "always launch through the helper" is not a gate: the next spec can still
 * reach for `electron.launch` directly and nothing will notice.
 *
 * So this asserts the one property that keeps the invariant true — every spec
 * that starts the desktop app starts it through `launchApp` — and it lives in
 * `src/` because vitest's `include` is `src/**` and would never see a test filed
 * beside the specs it guards. That is the same shape as `styles/measures.test.ts`
 * (a CSS declaration asserted from source) and the Rust
 * `autovis_cdn_desktop_contract` test: the assertion goes where the runner
 * already looks, and reads the file it is really about.
 *
 * Adding an allowed exception is a deliberate act. Each one below names a file
 * that launches something OTHER than the desktop app, and says why it needs no
 * config root.
 */

import { describe, expect, it } from 'vitest';
import { readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';

const E2E_DIR = join(__dirname, '../../tests/e2e');

/**
 * Files permitted to call `electron.launch` directly.
 *
 * `helpers/app.ts` IS the sanctioned launcher. The layout fixture starts a
 * throwaway main process (`user-message-layout.main.cjs`) that renders a static
 * HTML fixture — no daemon, no config, nothing to sandbox.
 */
const ALLOWED_DIRECT_LAUNCH = new Set(['helpers/app.ts', 'user-message-layout.spec.ts']);

/**
 * Specs skipped wholesale because the UI they drive was removed. They still
 * contain `electron.launch` calls that would run unsandboxed if re-enabled, so
 * they are listed here rather than silently tolerated — whoever un-skips them
 * has to deal with this test, which is the point.
 */
const SKIPPED_LEGACY = new Set([
  'context-management.spec.ts',
  'enhanced-context-management.spec.ts',
]);

function specFiles(): string[] {
  const out: string[] = [];
  for (const entry of readdirSync(E2E_DIR, { withFileTypes: true })) {
    if (entry.isDirectory()) {
      for (const child of readdirSync(join(E2E_DIR, entry.name))) {
        if (child.endsWith('.ts')) out.push(`${entry.name}/${child}`);
      }
    } else if (entry.name.endsWith('.ts')) {
      out.push(entry.name);
    }
  }
  return out.sort();
}

const read = (rel: string) => readFileSync(join(E2E_DIR, rel), 'utf8');

/**
 * The file with its comments removed.
 *
 * Necessary, not tidy: these files *discuss* `electron.launch` at length —
 * `schedule-artifact.helpers.ts` explains in its header why the packaged app
 * cannot use it — and matching prose would report a docblock as a violation.
 */
function code(rel: string): string {
  return read(rel)
    .replace(/\/\*[\s\S]*?\*\//g, '')
    .replace(/(^|[^:])\/\/.*$/gm, '$1');
}

describe('the e2e suite cannot launch the app outside a sandbox', () => {
  it('no spec calls electron.launch except the sanctioned launcher and the fixture', () => {
    const offenders = specFiles().filter(
      (rel) =>
        !ALLOWED_DIRECT_LAUNCH.has(rel) &&
        !SKIPPED_LEGACY.has(rel) &&
        /\belectron\.launch\s*\(/.test(code(rel))
    );

    expect(
      offenders,
      'These files launch Electron directly instead of through `launchApp` in ' +
        'tests/e2e/helpers/app.ts, which is what creates the config sandbox. An ' +
        'unsandboxed launch runs against the developer’s real ~/.config/biorouter ' +
        'and has been measured writing to it. Route the launch through `launchApp`, ' +
        'or add the file to ALLOWED_DIRECT_LAUNCH with the reason it needs no root.'
    ).toEqual([]);
  });

  it('every legacy spec on the exception list is actually skipped', () => {
    // The exception above is only defensible while these files cannot run. If one
    // is re-enabled, it must be sandboxed first.
    for (const rel of SKIPPED_LEGACY) {
      expect(read(rel), `${rel} is exempted from the launch rule but is no longer skipped`).toMatch(
        /test\.skip\(\s*true|describe\.skip/
      );
    }
  });

  it('the sanctioned launcher always supplies a config root and its own profile', () => {
    const app = read('helpers/app.ts');
    // The two halves of the invariant: the daemon/config redirect, and the
    // renderer profile. Losing either one silently re-shares the developer's state.
    expect(app).toMatch(/BIOROUTER_PATH_ROOT: sandbox\.root/);
    expect(app).toMatch(/--user-data-dir=\$\{path\.join\(sandbox\.root, 'electron'\)\}/);
    // A sandbox is created when the caller does not supply one, so a bare
    // `launchApp()` is still sandboxed.
    expect(app).toMatch(/options\.sandbox \?\? createSandbox\(\)/);
  });
});
