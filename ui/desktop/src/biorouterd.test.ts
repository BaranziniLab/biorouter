// @vitest-environment node

import type { App } from 'electron';
import type { PathLike, Stats } from 'node:fs';
import { afterAll, beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({
  logInfo: vi.fn(),
  logError: vi.fn(),
  logWarn: vi.fn(),
  logDebug: vi.fn(),
  spawn: vi.fn(),
}));

// This test exercises env-passing and secret-redaction, not the real binary.
// `getBiorouterdBinaryPath` does a live filesystem probe for a compiled
// `biorouterd`, which exists in a normal dev tree (`target/debug/`) but NOT in
// an isolated build (e.g. a custom CARGO_TARGET_DIR) or a fresh CI checkout —
// there the probe throws and the test fails for reasons unrelated to what it
// asserts. Mock `node:fs` so the probe resolves the first candidate path
// without needing an artifact on disk; spawn is already mocked, so no binary is
// ever executed. `fs` is used *only* by the binary-path resolver here, so this
// leaves all other behavior intact.
const isBiorouterdPath = (p: PathLike): boolean => {
  const s = p.toString();
  return s.endsWith('biorouterd') || s.endsWith('biorouterd.exe');
};
vi.mock('node:fs', async (importOriginal) => {
  const actual = await importOriginal<typeof import('node:fs')>();
  const existsSync = (p: PathLike): boolean => (isBiorouterdPath(p) ? true : actual.existsSync(p));
  const statSync = (p: PathLike): Stats =>
    isBiorouterdPath(p) ? ({ isFile: () => true } as unknown as Stats) : actual.statSync(p);
  const mocked = { ...actual, existsSync, statSync };
  return { ...mocked, default: mocked };
});

// The stderr handler routes each line at the daemon's own severity, so it can
// call any of these four. A mock missing `warn`/`debug` would not fail today
// (no test here drives the stderr handler) but would surface as a confusing
// `log.warn is not a function` the first time one does.
vi.mock('./utils/logger', () => ({
  default: {
    info: mocks.logInfo,
    error: mocks.logError,
    warn: mocks.logWarn,
    debug: mocks.logDebug,
  },
}));

vi.mock('child_process', async (importOriginal) => {
  const actual = await importOriginal<typeof import('child_process')>();
  const mocked = {
    ...actual,
    spawn: mocks.spawn,
  };
  return {
    ...mocked,
    default: mocked,
  };
});

import { createHash } from 'node:crypto';

import { externalBackendUrlFromEnv, startBiorouterd } from './biorouterd';

const sha256Hex = (value: string): string => createHash('sha256').update(value).digest('hex');

describe('startBiorouterd logging', () => {
  const inheritedKey = 'BIOROUTER_TEST_INHERITED_VALUE';
  const inheritedValue = 'inherited-value-sentinel';
  const overrideValue = 'override-value-sentinel';
  const serverSecret = 'server-secret-sentinel';
  const userActionKeyForTest = 'user-action-key-sentinel';
  const previousInheritedValue = process.env[inheritedKey];
  let stdinWrites: string[] = [];
  let stdinEnded = false;

  beforeEach(() => {
    process.env[inheritedKey] = inheritedValue;
    mocks.logInfo.mockClear();
    mocks.logError.mockClear();
    mocks.logWarn.mockClear();
    mocks.logDebug.mockClear();
    mocks.spawn.mockReset();
    stdinWrites = [];
    stdinEnded = false;
    mocks.spawn.mockReturnValue({
      stdin: {
        write: vi.fn((chunk: string) => {
          stdinWrites.push(String(chunk));
          return true;
        }),
        end: vi.fn(() => {
          stdinEnded = true;
        }),
      },
      stdout: { on: vi.fn() },
      stderr: { on: vi.fn() },
      on: vi.fn(),
      kill: vi.fn(),
      unref: vi.fn(),
    });
  });

  // Issue #56 / AR-11, asserted rather than reviewed: the environment and argv
  // of this process are both recoverable IN-PROCESS by any tool that reads a
  // caller-named path (`/proc/self/environ`) or, on macOS, by
  // `sysctl(KERN_PROCARGS2)`, which no sandbox profile can gate. A user-proof
  // delivered that way is a proof the model already holds.
  it('hands the user-action digest on stdin and never through the environment', async () => {
    const app = {
      isPackaged: false,
      on: vi.fn(),
    } as unknown as App;

    await startBiorouterd({
      app,
      serverSecret,
      userActionKey: userActionKeyForTest,
      dir: process.cwd(),
    });

    const spawnArgs = {
      args: mocks.spawn.mock.calls[0]?.[1] as string[],
      options: mocks.spawn.mock.calls[0]?.[2] as {
        stdio: string[];
        env: Record<string, string>;
      },
    };
    expect(spawnArgs.options.stdio[0]).toBe('pipe'); // was 'ignore'
    expect(spawnArgs.args).toEqual(['agent']); // not on argv either
    const env = spawnArgs.options.env;
    // SD-12 added ONE user-action-shaped variable, and it is not a secret: it is
    // the launcher declaring that it sends a digest at all, so a daemon that gets
    // none can tell a fault from a `biorouter serve` deployment. Exempted by name
    // and pinned to a boolean below, rather than by loosening the pattern — the
    // property being defended is that nothing DERIVED FROM THE KEY travels here,
    // and a regex that stopped matching `USER_ACTION` would stop defending it.
    expect(env.BIOROUTER_USER_ACTION_EXPECTED).toBe('1');
    for (const [k, v] of Object.entries(env)) {
      if (k === 'BIOROUTER_USER_ACTION_EXPECTED') continue;
      expect(k).not.toMatch(/USER_ACTION/i);
      expect(v).not.toBe(userActionKeyForTest); // and not smuggled under another name
    }
    // The declaration carries nothing recoverable: not the key, not its digest.
    expect(env.BIOROUTER_USER_ACTION_EXPECTED).not.toContain(userActionKeyForTest);
    expect(env.BIOROUTER_USER_ACTION_EXPECTED).not.toContain(sha256Hex(userActionKeyForTest));
    expect(stdinWrites.join('')).toContain(sha256Hex(userActionKeyForTest));
    expect(stdinWrites.join('')).not.toContain(userActionKeyForTest); // digest, never the key
    expect(stdinEnded).toBe(true);
  });

  // Finding 3 of the 2026-09-12 review: the declaration is what separates "no key
  // was ever meant to arrive" from "one was and did not", so it must survive the
  // case where the launcher has no key to send — the only case where the daemon
  // needs it. A `if (userActionKey)` around it would delete the signal exactly
  // there.
  it('still declares that it sends a key when it has none to send', async () => {
    const app = {
      isPackaged: false,
      on: vi.fn(),
    } as unknown as App;

    await startBiorouterd({ app, serverSecret, dir: process.cwd() });

    const options = mocks.spawn.mock.calls[0]?.[2] as { env: Record<string, string> };
    expect(options.env.BIOROUTER_USER_ACTION_EXPECTED).toBe('1');
    expect(stdinWrites.join('')).toBe(''); // nothing to write
    expect(stdinEnded).toBe(true);
  });

  afterAll(() => {
    if (previousInheritedValue === undefined) {
      delete process.env[inheritedKey];
    } else {
      process.env[inheritedKey] = previousInheritedValue;
    }
  });

  it('passes environment values to the child without writing them to logs', async () => {
    const app = {
      isPackaged: false,
      on: vi.fn(),
    } as unknown as App;

    await startBiorouterd({
      app,
      serverSecret,
      dir: process.cwd(),
      env: { BIOROUTER_TEST_OVERRIDE_VALUE: overrideValue },
    });

    const spawnOptions = mocks.spawn.mock.calls[0]?.[2];
    expect(spawnOptions?.env?.[inheritedKey]).toBe(inheritedValue);
    expect(spawnOptions?.env?.BIOROUTER_TEST_OVERRIDE_VALUE).toBe(overrideValue);
    expect(spawnOptions?.env?.BIOROUTER_SERVER__SECRET_KEY).toBe(serverSecret);

    const logged = JSON.stringify(mocks.logInfo.mock.calls);
    expect(logged).not.toContain(inheritedValue);
    expect(logged).not.toContain(overrideValue);
    expect(logged).not.toContain(serverSecret);
  });
});

/**
 * Issue #56 — the External Backend path is the supported way for the `biorouter`
 * CLI to reach a daemon the desktop app is also using, so the developer escape
 * hatch that shares its code must honour the port this repo documents.
 *
 * The URL used to be a hard-coded `http://127.0.0.1:3000`, so a developer who
 * moved their daemon (which `just debug-server` and `BIOROUTER_EXTERNAL_PORT`
 * both invite) had the app silently connect to 3000 and report the backend as
 * down.
 */
describe('externalBackendUrlFromEnv', () => {
  it('honours BIOROUTER_EXTERNAL_PORT', () => {
    expect(externalBackendUrlFromEnv({ BIOROUTER_EXTERNAL_PORT: '3456' })).toBe(
      'http://127.0.0.1:3456'
    );
  });

  it('falls back to 3000 when the port is absent, blank or not a usable port', () => {
    // A malformed value must not compose a URL that cannot resolve — a wrong
    // port that at least exists is diagnosable, `http://127.0.0.1:NaN` is not.
    for (const value of [undefined, '', '   ', 'abc', '0', '-1', '70000', '12.5']) {
      expect(
        externalBackendUrlFromEnv(value === undefined ? {} : { BIOROUTER_EXTERNAL_PORT: value })
      ).toBe('http://127.0.0.1:3000');
    }
  });
});
