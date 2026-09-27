// @vitest-environment node
import { EventEmitter } from 'node:events';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { afterEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({ spawn: vi.fn() }));

vi.mock('node:child_process', async (importOriginal) => {
  const actual = await importOriginal<typeof import('node:child_process')>();
  const mocked = { ...actual, spawn: mocks.spawn };
  return { ...mocked, default: mocked };
});

vi.mock('node:fs', async (importOriginal) => {
  const actual = await importOriginal<typeof import('node:fs')>();
  const mocked = {
    ...actual,
    accessSync: (location: string, mode?: number) => {
      if (
        process.platform !== 'darwin' &&
        process.platform !== 'win32' &&
        location === '/usr/bin/zenity'
      )
        return undefined;
      return actual.accessSync(location, mode);
    },
    statSync: (...args: Parameters<typeof actual.statSync>) => {
      const location = args[0];
      if (
        process.platform !== 'darwin' &&
        process.platform !== 'win32' &&
        location === '/usr/bin/zenity'
      )
        return { isFile: () => true } as ReturnType<typeof actual.statSync>;
      return actual.statSync(...args);
    },
  };
  return { ...mocked, default: mocked };
});

import { promptNativeSecret } from './nativeSecretPrompt';

class FakeChild extends EventEmitter {
  stdout = new EventEmitter();
  kill = vi.fn();
}

describe('native shared-daemon approval prompt', () => {
  afterEach(() => {
    vi.useRealTimers();
    vi.restoreAllMocks();
    mocks.spawn.mockReset();
    delete process.env.BIOROUTER_TEST_PASSWORD;
  });

  it('returns the hidden prompt answer while keeping it out of argv and filtered environment', async () => {
    const child = new FakeChild();
    mocks.spawn.mockReturnValue(child);
    const secret = 'approval-secret-for-test';
    process.env.BIOROUTER_TEST_PASSWORD = secret;

    const pending = promptNativeSecret('Shared daemon approval', 'Enter the approval secret');
    expect(mocks.spawn).toHaveBeenCalledOnce();
    const [program, args, options] = mocks.spawn.mock.calls[0] as [
      string,
      string[],
      { env: Record<string, string>; stdio: string[] },
    ];
    expect(program).toBe(
      process.platform === 'darwin'
        ? '/usr/bin/osascript'
        : process.platform === 'win32'
          ? 'powershell.exe'
          : '/usr/bin/zenity'
    );
    expect(args.join('\n')).not.toContain(secret);
    expect(options.env.BIOROUTER_TEST_PASSWORD).toBeUndefined();
    expect(options.stdio).toEqual(['ignore', 'pipe', 'ignore']);
    child.stdout.emit('data', Buffer.from(secret + '\n'));
    child.emit('close', 0);
    expect(await pending).toBe(secret);
  });

  it('activates the current application before showing the hidden-answer dialog on macOS', async () => {
    vi.spyOn(process, 'platform', 'get').mockReturnValue('darwin');
    const child = new FakeChild();
    mocks.spawn.mockReturnValue(child);
    const pending = promptNativeSecret('Crew', 'Approval');
    const [, args] = mocks.spawn.mock.calls[0] as [string, string[]];
    expect(args.slice(0, 3)).toEqual(['-e', 'tell current application to activate', '-e']);
    expect(args[3]).toContain('with hidden answer');
    child.stdout.emit('data', Buffer.from('secret\n'));
    child.emit('close', 0);
    await expect(pending).resolves.toBe('secret');
  });

  it('times out at 180 seconds, rejects late stdout, and resets the active guard', async () => {
    vi.useFakeTimers();
    const first = new FakeChild();
    const second = new FakeChild();
    mocks.spawn.mockReturnValueOnce(first).mockReturnValueOnce(second);
    const pending = promptNativeSecret('Crew', 'Approval');
    await vi.advanceTimersByTimeAsync(180_000);
    expect(first.kill).toHaveBeenCalledTimes(1);
    first.stdout.emit('data', Buffer.from('late-secret\n'));
    first.emit('close', 0);
    await expect(pending).rejects.toThrow(/timed out after three minutes/);

    const next = promptNativeSecret('Crew', 'Approval again');
    second.stdout.emit('data', Buffer.from('fresh-secret\n'));
    second.emit('close', 0);
    await expect(next).resolves.toBe('fresh-secret');
    expect(mocks.spawn).toHaveBeenCalledTimes(2);
  });

  it('maps native dialog cancellation to no secret and clears the active prompt guard', async () => {
    const child = new FakeChild();
    mocks.spawn.mockReturnValue(child);
    const pending = promptNativeSecret('title', 'message');
    child.emit('close', 1);
    await expect(pending).resolves.toBeUndefined();

    const second = new FakeChild();
    mocks.spawn.mockReturnValue(second);
    const retry = promptNativeSecret('title', 'message');
    second.emit('close', 1);
    await expect(retry).resolves.toBeUndefined();
    expect(mocks.spawn).toHaveBeenCalledTimes(2);
  });

  it('kills and rejects an oversized native response before returning it', async () => {
    const child = new FakeChild();
    mocks.spawn.mockReturnValue(child);
    const pending = promptNativeSecret('title', 'message');
    child.stdout.emit('data', Buffer.alloc(4099, 0x61));
    expect(child.kill).toHaveBeenCalledOnce();
    child.emit('close', 1);
    await expect(pending).rejects.toThrow('exceeds the allowed length');
  });
});

/**
 * DOCS-6: the brand is "Biorouter", lowercase r. The approval-secret prompts said "BioRouter", so
 * the manual had to quote a spelling it uses nowhere else, in the one dialog it tells people to
 * trust with a secret. Read at the source: the prompts are native dialogs no test renders.
 */
describe('native approval prompts spell the brand "Biorouter"', () => {
  const source = (name: string) => readFileSync(join(__dirname, name), 'utf8');
  /** Every string literal in `code`, comments left out. */
  const literals = (code: string) =>
    [
      ...code
        .replace(/\/\*[\s\S]*?\*\//g, '')
        .replace(/^\s*\/\/.*$/gm, '')
        .matchAll(/'(?:[^'\\\n]|\\.)*'|"(?:[^"\\\n]|\\.)*"|`(?:[^`\\]|\\.)*`/g),
    ].map((match) => match[0]);

  it('in every title and message main.ts passes to promptNativeSecret', () => {
    const calls = [...source('main.ts').matchAll(/promptNativeSecret\(([\s\S]*?)\);/g)].map(
      (match) => match[1]
    );
    expect(calls.length).toBeGreaterThanOrEqual(4);
    const texts = calls.flatMap(literals);
    expect(texts.filter((text) => text.includes('BioRouter'))).toEqual([]);
    expect(texts.filter((text) => text.includes('Biorouter')).length).toBeGreaterThanOrEqual(3);
  });

  it('in every sentence nativeSecretPrompt.ts shows', () => {
    const texts = literals(source('nativeSecretPrompt.ts'));
    expect(texts.length).toBeGreaterThan(5);
    expect(texts.filter((text) => text.includes('BioRouter'))).toEqual([]);
  });
});
