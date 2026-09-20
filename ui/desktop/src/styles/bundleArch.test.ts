import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';

/**
 * A bundle that declares a build target to `forge.config.ts` must declare it on
 * the command that EVALUATES that config, which is `npm run make`.
 *
 * ⚠ The trap is shell scoping, not the config. `VAR=value cmd` sets the variable
 * for `cmd` alone. `bundle:intel` set `ELECTRON_ARCH=x64` on the two node
 * scripts that precede make and not on make itself, so the config fell back to
 * `process.arch` — arm64 on the machine that builds releases — and the Intel app
 * shipped the `darwin-arm64` node-pty prebuild.
 *
 * That does not fail loudly. node-pty cannot load a foreign-architecture native
 * module and silently falls back to pipes, so the app launches and the terminal
 * is quietly degraded. `verify-packaged-dependencies.js` catches it at package
 * time, which is how it was found — but only once somebody builds Intel, which
 * happens on a release and nowhere else.
 *
 * The rule is written as "every variable this script sets anywhere must also be
 * set on make", rather than naming Intel, because the bug is the shell scoping
 * and it can recur on any target. Scripts are read from package.json: a test
 * carrying its own copy of a command passes while the real one drifts.
 */
describe('bundle scripts declare their build target on the make invocation', () => {
  const scripts: Record<string, string> = JSON.parse(
    readFileSync(join(__dirname, '../../package.json'), 'utf8')
  ).scripts;

  const FORGE_VARS = /\b(ELECTRON_ARCH|ELECTRON_PLATFORM)=(\S+)/g;

  const declaring = Object.entries(scripts).filter(
    ([name, body]) =>
      name.startsWith('bundle:') &&
      /\bnpm run make\b/.test(body) &&
      [...body.matchAll(FORGE_VARS)].length > 0
  );

  it('finds the bundles it means to check', () => {
    // A filter that matched nothing would make every assertion below vacuous.
    expect(declaring.map(([name]) => name).sort()).toEqual([
      'bundle:intel',
      'bundle:intel-dmg',
      'bundle:linux',
    ]);
  });

  it.each(declaring)('%s carries its forge variables into make', (_name, body) => {
    const declared = new Map([...body.matchAll(FORGE_VARS)].map((m) => [m[1], m[2]]));
    const makes = body.split('&&').filter((part) => /\bnpm run make\b/.test(part));
    expect(makes.length).toBeGreaterThan(0);
    for (const part of makes) {
      const onThisCommand = new Map([...part.matchAll(FORGE_VARS)].map((m) => [m[1], m[2]]));
      for (const [name, value] of declared) {
        expect(onThisCommand.get(name), `${name} must be set on the make command itself`).toBe(
          value
        );
      }
    }
  });
});
