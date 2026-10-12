// @vitest-environment node
import { execFileSync } from 'node:child_process';
import { existsSync, readdirSync, readFileSync } from 'node:fs';
import { join, relative } from 'node:path';
import { describe, expect, it } from 'vitest';
import { HOOK_TIMEOUT_MS, TEST_TIMEOUT_MS } from './timeouts';

/**
 * The hook budget the Chromium-backed files depend on, pinned.
 *
 * About eighty `*.browser.test.*` files launch their own Chromium in a
 * `beforeAll` and close it in an `afterAll`. Under a full run on a loaded
 * machine the close took longer than the old 30 s hook budget in two to four
 * files per run, with every assertion in them passed (`Hook timed out in
 * 30000ms`). The budget is ./timeouts.ts's `HOOK_TIMEOUT_MS`; this file keeps
 * the three places that could silently put it back from doing so:
 *
 *   - vitest.config.ts must resolve `hookTimeout` to that constant (a literal
 *     there, or an `export default` that stops reading it, would not).
 *   - A `--hookTimeout` flag beats the config. The `unit` job in frontend.yml
 *     carried `--hookTimeout=30000` until this budget moved, so CI would have
 *     kept the old number whatever the config said. No script or workflow step
 *     may pass one lower than the constant.
 *   - The constant must stay above the test budget. A hook that is allowed
 *     less time than the test it serves is the shape of the original failure.
 *
 * The config is resolved in a child `node` for the reason vitestIsolation.test.ts
 * gives: this worker runs under the very config being checked.
 */

const DESKTOP = join(__dirname, '../..');
const REPO = join(DESKTOP, '../..');

/**
 * The smallest hook budget that has held. A full run of the whole suite at load
 * average 115 to 300 overran 30 s, and one at 560 overran 120 s; a lower number
 * than this is a regression to be argued, not a tidy-up.
 */
const FLOOR_MS = 300_000;

const MARKER = '@@hook-timeout-guard@@';

const RESOLVE_IN_CHILD = `
const { parseCLI, resolveConfig } = await import('vitest/node');
const { options } = parseCLI('vitest run');
const { vitestConfig } = await resolveConfig({ ...options, root: process.cwd() });
process.stdout.write('\\n${MARKER}' + JSON.stringify({
  hookTimeout: vitestConfig.hookTimeout,
  testTimeout: vitestConfig.testTimeout,
}) + '\\n');
`;

function resolveConfigInChild(): { hookTimeout: number; testTimeout: number } {
  const env = Object.fromEntries(
    Object.entries(process.env).filter(([key]) => key !== 'VITEST' && !key.startsWith('VITEST_'))
  );
  const stdout = execFileSync(process.execPath, ['--input-type=module', '-e', RESOLVE_IN_CHILD], {
    cwd: DESKTOP,
    env,
    encoding: 'utf8',
  });
  const line = stdout.split('\n').find((l) => l.startsWith(MARKER));
  if (!line) throw new Error(`the resolver printed no result:\n${stdout}`);
  return JSON.parse(line.slice(MARKER.length));
}

/** Every file that can carry a vitest command line: package scripts, workflows, composite actions. */
function commandFiles(): { where: string; body: string }[] {
  const files: { where: string; body: string }[] = [
    { where: 'ui/desktop/package.json', body: readFileSync(join(DESKTOP, 'package.json'), 'utf8') },
  ];
  const workflows = join(REPO, '.github/workflows');
  for (const file of readdirSync(workflows)) {
    if (/\.ya?ml$/.test(file)) {
      files.push({
        where: relative(REPO, join(workflows, file)),
        body: readFileSync(join(workflows, file), 'utf8'),
      });
    }
  }
  const actions = join(REPO, '.github/actions');
  if (existsSync(actions)) {
    for (const dir of readdirSync(actions)) {
      for (const name of ['action.yml', 'action.yaml']) {
        const path = join(actions, dir, name);
        if (existsSync(path))
          files.push({ where: relative(REPO, path), body: readFileSync(path, 'utf8') });
      }
    }
  }
  const justfile = join(REPO, 'Justfile');
  if (existsSync(justfile)) files.push({ where: 'Justfile', body: readFileSync(justfile, 'utf8') });
  return files;
}

describe('the hook budget the Chromium-backed files rely on', () => {
  it('is no lower than the floor that held in a loaded full run, and above the test budget', () => {
    expect(HOOK_TIMEOUT_MS).toBeGreaterThanOrEqual(FLOOR_MS);
    expect(HOOK_TIMEOUT_MS).toBeGreaterThan(TEST_TIMEOUT_MS);
  });

  it('is what vitest.config.ts resolves, and the test budget is untouched', () => {
    const resolved = resolveConfigInChild();
    expect(resolved.hookTimeout, 'vitest.config.ts resolves a different hookTimeout').toBe(
      HOOK_TIMEOUT_MS
    );
    expect(resolved.testTimeout).toBe(TEST_TIMEOUT_MS);
  });

  it('is not undercut by a --hookTimeout flag in any script, workflow, action or Justfile', () => {
    const undercut: string[] = [];
    for (const { where, body } of commandFiles()) {
      // `--hookTimeout=N`, `--hookTimeout N` and the kebab spelling cac also reads.
      for (const match of body.matchAll(/--hook-?timeout(?:=|\s+)(\d+)/gi)) {
        if (Number(match[1]) < HOOK_TIMEOUT_MS) {
          undercut.push(`${where}: ${match[0]} (the config says ${HOOK_TIMEOUT_MS})`);
        }
      }
    }
    expect(
      undercut,
      `a command line beats vitest.config.ts, so these keep the old hook budget: ${undercut.join('; ')}`
    ).toEqual([]);
  });

  it('finds the unit job it is guarding (frontend.yml still runs vitest)', () => {
    const frontend = commandFiles().find((f) => f.where === '.github/workflows/frontend.yml');
    expect(frontend, 'frontend.yml is not among the files scanned').toBeDefined();
    expect(frontend?.body).toMatch(/npx vitest run/);
  });
});
