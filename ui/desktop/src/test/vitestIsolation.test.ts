// @vitest-environment node
import { execFileSync } from 'node:child_process';
import { existsSync, readdirSync, readFileSync } from 'node:fs';
import { join, relative } from 'node:path';
import { load } from 'js-yaml';
import { describe, expect, it } from 'vitest';

/**
 * The precondition under ./queuedTimers.ts's "never in a later file", pinned.
 *
 * That file argues that a timer longer than the drain's 1 ms, still queued when
 * a spec file ends, can fire only inside that file's own worker and never
 * inside a later file. The argument holds only while every file gets a worker
 * of its own and the pool ends it after the file (vitest 4.0.18,
 * dist/chunks/cli-api.B7PN_QUv.js: a new runner per file at 8016 and 8117/8119,
 * shared only when `isolate` is false at 8057 and 8100; the forks worker is
 * stopped with SIGTERM and then SIGKILL at 7696-7697, a threads worker with
 * `thread.terminate()` at 7759). Two settings break it:
 *
 *   - `isolate: false` (`--no-isolate`): the pool hands the next file to the
 *     same worker. Measured with `--no-isolate` as the control (recorded in
 *     ./queuedTimers.ts): a 5000 ms timer queued as one file's last act fired
 *     inside the next file, in the same pid.
 *   - `pool: 'vmThreads'` or `'vmForks'`: vitest forces `isolate` to false for
 *     both (dist/chunks/coverage.AVPTjMgw.js:2639), so files share a worker
 *     and are separated only by vm contexts inside it.
 *
 * Neither is set today, so the defaults apply: `pool: 'forks'`
 * (coverage.AVPTjMgw.js:2478) and `isolate: true`
 * (dist/chunks/defaults.BOqNVLsY.js:39). Nothing else would notice if that
 * changed, and the drain in ./setup.ts would keep passing while a long timer
 * started landing in the next file's tests. So this resolves the config
 * through vitest's own public `resolveConfig` (vitest/node), once for the
 * config file alone and once for every command in the repository that runs
 * vitest, with that command's own flags parsed by vitest's `parseCLI`: a flag
 * is judged by what vitest makes of it. The last test adds a plain scan for the
 * flags' spellings, for a command the reader here does not recognise as vitest.
 *
 * Shown failing, each for its own reason, before this landed: `isolate: false`
 * and `pool: 'vmThreads'` in vitest.config.ts, `--no-isolate` on frontend.yml's
 * `npx vitest run`, `--pool=vmForks` on `test:run`, `npm run test:run --
 * --no-isolate` in a workflow step, a `projects` entry, `pnpm exec vitest` in
 * the CI step (unreadable), and the vitest command removed from the unit job.
 */

const DESKTOP = join(__dirname, '../..');
const REPO = join(DESKTOP, '../..');

/** The pools that give each file a worker of its own when `isolate` is on. */
const PER_FILE_POOLS = ['forks', 'threads'];

const MARKER = '@@vitest-isolation-guard@@';

interface Invocation {
  /** Where the command lives, for the failure message. */
  where: string;
  /** The command from `vitest` onwards, as `parseCLI` takes it. */
  argv: string;
}

/**
 * A shell script split into simple commands: line continuations joined, then
 * split at newlines, `&&`, `||`, `;` and `|`. Enough for the one-line commands
 * in package.json and a workflow's `run:` blocks; a command it cannot read is a
 * failure below, not a silent pass.
 */
function simpleCommands(script: string): string[] {
  return script
    .replace(/\\\r?\n/g, ' ')
    .split(/\r?\n|&&|\|\||[;|]/)
    .map((command) => command.trim().replace(/\s+/g, ' '))
    .filter((command) => command !== '' && !command.startsWith('#'));
}

/** Leading `NAME=value` environment assignments, which do not change the command. */
const ENV_PREFIX = /^(?:[A-Za-z_][A-Za-z0-9_]*=\S*\s+)*/;

/** The word `vitest` used as a command or a path to one, not `vitest-junit.xml`. */
const MENTIONS_VITEST = /(?:^|[\s/'"])vitest(?:\.mjs)?(?=\s|$)/;

const packageScripts: Record<string, string> = JSON.parse(
  readFileSync(join(DESKTOP, 'package.json'), 'utf8')
).scripts;

/**
 * The vitest invocations one simple command makes, or `null` when it names
 * vitest in a form this guard does not know how to read.
 */
function vitestInvocations(command: string, where: string, depth = 0): Invocation[] | null {
  const words = command.replace(ENV_PREFIX, '');
  const direct = /^(?:npx (?:--yes |-y |--no-install )?)?vitest(?= |$)(.*)$/.exec(words);
  if (direct) return [{ where, argv: `vitest${direct[1]}` }];

  const npmScript = /^npm (?:run(?:-script)? (\S+)|(test|t))(?: -- (.*))?$/.exec(words);
  if (npmScript) {
    const name = npmScript[1] ?? 'test';
    const extra = npmScript[3] ? ` ${npmScript[3]}` : '';
    const body = packageScripts[name];
    if (body === undefined) return [];
    // A script that reaches itself again is not one this guard can read.
    if (depth > 8) return null;
    const inner = simpleCommands(body).map((c) =>
      vitestInvocations(c, `${where} → package.json scripts["${name}"]`, depth + 1)
    );
    if (inner.some((found) => found === null)) return null;
    return inner.flatMap((found) => found ?? []).map((inv) => ({ ...inv, argv: inv.argv + extra }));
  }

  return MENTIONS_VITEST.test(words) ? null : [];
}

interface Step {
  name?: string;
  run?: string;
}

interface Collected {
  invocations: Invocation[];
  /** Commands that name vitest in a form `vitestInvocations` cannot read. */
  unreadable: string[];
  /** Every script body and `run:` block scanned, for the flag scan below. */
  scripts: { where: string; body: string }[];
}

function collect(): Collected {
  const out: Collected = { invocations: [], unreadable: [], scripts: [] };
  const scan = (where: string, body: string) => {
    out.scripts.push({ where, body });
    for (const command of simpleCommands(body)) {
      const found = vitestInvocations(command, where);
      if (found === null) out.unreadable.push(`${where}: ${command}`);
      else out.invocations.push(...found);
    }
  };

  for (const [name, body] of Object.entries(packageScripts)) {
    // Only a script that runs vitest itself: one that reaches it through
    // `npm run` is read when a workflow calls that script.
    for (const command of simpleCommands(body)) {
      const found = vitestInvocations(command, `package.json scripts["${name}"]`);
      if (found === null) out.unreadable.push(`package.json scripts["${name}"]: ${command}`);
      else if (!/^npm /.test(command.replace(ENV_PREFIX, ''))) out.invocations.push(...found);
    }
    out.scripts.push({ where: `package.json scripts["${name}"]`, body });
  }

  const yamlFiles: string[] = [];
  const workflows = join(REPO, '.github/workflows');
  for (const file of readdirSync(workflows)) {
    if (/\.ya?ml$/.test(file)) yamlFiles.push(join(workflows, file));
  }
  const actions = join(REPO, '.github/actions');
  if (existsSync(actions)) {
    for (const dir of readdirSync(actions)) {
      for (const file of ['action.yml', 'action.yaml']) {
        if (existsSync(join(actions, dir, file))) yamlFiles.push(join(actions, dir, file));
      }
    }
  }

  for (const file of yamlFiles) {
    const doc = load(readFileSync(file, 'utf8')) as {
      jobs?: Record<string, { steps?: Step[] }>;
      runs?: { steps?: Step[] };
    };
    const rel = relative(REPO, file);
    const stepLists: [string, Step[]][] = [
      ...Object.entries(doc.jobs ?? {}).map(([job, spec]): [string, Step[]] => [
        `${rel} › ${job}`,
        spec.steps ?? [],
      ]),
      [rel, doc.runs?.steps ?? []],
    ];
    for (const [prefix, steps] of stepLists) {
      steps.forEach((step, index) => {
        if (typeof step.run === 'string')
          scan(`${prefix} › ${step.name ?? `step ${index}`}`, step.run);
      });
    }
  }
  return out;
}

interface Resolved {
  argv: string;
  pool: string;
  isolate: boolean | undefined;
  projectCount: number;
}

/**
 * Resolves each command's config in a child `node`, not in this worker. The
 * worker is one the config under test chose: with `pool: 'vmThreads'` set, vite
 * could not load vitest.config.ts from inside it at all ("Cannot use import
 * statement outside a module"), so the guard failed with a message about the
 * wrong thing. A plain child sees the config exactly as `npx vitest` does.
 */
const RESOLVE_IN_CHILD = `
const { parseCLI, resolveConfig } = await import('vitest/node');
const out = [];
for (const argv of JSON.parse(process.env.VITEST_ISOLATION_GUARD_ARGVS)) {
  const { options } = parseCLI(argv);
  const { vitestConfig } = await resolveConfig({ ...options, root: process.cwd() });
  out.push({
    argv,
    pool: vitestConfig.pool,
    isolate: vitestConfig.isolate,
    projectCount: (vitestConfig.projects ?? []).length,
  });
}
process.stdout.write('\\n${MARKER}' + JSON.stringify(out) + '\\n');
`;

function resolveInChild(argvs: string[]): Resolved[] {
  // This worker's own VITEST_* variables describe the run that is executing
  // this test, not the one being resolved.
  const env = Object.fromEntries(
    Object.entries(process.env).filter(([key]) => key !== 'VITEST' && !key.startsWith('VITEST_'))
  );
  const stdout = execFileSync(process.execPath, ['--input-type=module', '-e', RESOLVE_IN_CHILD], {
    cwd: DESKTOP,
    env: { ...env, VITEST_ISOLATION_GUARD_ARGVS: JSON.stringify(argvs) },
    encoding: 'utf8',
  });
  const line = stdout.split('\n').find((l) => l.startsWith(MARKER));
  if (!line) throw new Error(`the resolver printed no result:\n${stdout}`);
  return JSON.parse(line.slice(MARKER.length));
}

describe('every file runs in a worker of its own (the precondition of ./queuedTimers.ts)', () => {
  const collected = collect();

  it('reads every command that runs vitest, and finds the CI job and test:run among them', () => {
    expect(
      collected.unreadable,
      `a command runs vitest in a form this guard cannot read; teach vitestInvocations it: ${collected.unreadable.join('; ')}`
    ).toEqual([]);
    // Not vacuous: the CI job and the documented local command are both here.
    const wheres = collected.invocations.map((inv) => inv.where);
    expect(wheres, `found: ${wheres.join('; ')}`).toContain('package.json scripts["test:run"]');
    expect(
      wheres.some((where) => where.startsWith('.github/workflows/frontend.yml › unit ›')),
      `frontend.yml's unit job runs no vitest command this guard can see; found: ${wheres.join('; ')}`
    ).toBe(true);
  });

  // One child for every command, the config file alone first, run from the
  // first test that needs it so a resolver failure is reported against a test.
  let resolved: Resolved[] | undefined;
  const resolveAll = () =>
    (resolved ??= resolveInChild(['vitest run', ...collected.invocations.map((inv) => inv.argv)]));

  it('resolves vitest.config.ts to a per-file pool with isolate on, and no projects', () => {
    const [configAlone] = resolveAll();
    expect(PER_FILE_POOLS, `vitest.config.ts resolves pool to "${configAlone.pool}"`).toContain(
      configAlone.pool
    );
    expect(configAlone.isolate, 'vitest.config.ts resolves isolate to false').not.toBe(false);
    // A project carries its own pool and isolate, which the root config above
    // does not describe (measured: a project with `isolate: false` left the
    // root resolving `true`). Resolve each project here before adding one.
    expect(
      configAlone.projectCount,
      'vitest.config.ts declares projects; this guard reads only the root'
    ).toBe(0);
  });

  it('keeps that for every command in the repository that runs vitest', () => {
    const perCommand = resolveAll().slice(1);
    collected.invocations.forEach((inv, i) => {
      const got = perCommand[i];
      expect(got.argv).toBe(inv.argv);
      expect(PER_FILE_POOLS, `${inv.where} (${inv.argv}) resolves pool to "${got.pool}"`).toContain(
        got.pool
      );
      expect(got.isolate, `${inv.where} (${inv.argv}) resolves isolate to false`).not.toBe(false);
    });
  });

  it('has no script or workflow step that passes --no-isolate or a vm pool, in any form', () => {
    // Belt and braces for a command the reader above does not recognise as
    // vitest at all (a wrapper script, `yarn`, `pnpm`): the flags themselves.
    const offending = collected.scripts
      .filter(({ body }) => /--no-isolate|--isolate[= ]false|--pool[= ]['"]?vm/i.test(body))
      .map(({ where }) => where);
    expect(offending, `passes --no-isolate or a vm pool: ${offending.join('; ')}`).toEqual([]);
  });
});
