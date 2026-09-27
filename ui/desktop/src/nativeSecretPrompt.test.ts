// @vitest-environment node
import { EventEmitter } from 'node:events';
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
