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

import {
  closeNativeSecretPrompt,
  nativePromptOutcome,
  promptNativeSecret,
} from './nativeSecretPrompt';

class FakeChild extends EventEmitter {
  stdout = new EventEmitter();
  stderr = new EventEmitter();
  exitCode: number | null = null;
  signalCode: string | null = null;
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
    const secret = 'vault-passphrase-for-test';
    process.env.BIOROUTER_TEST_PASSWORD = secret;

    const pending = promptNativeSecret('Unlock Crew encrypted vault', 'Enter the vault passphrase');
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
    // Linux reads zenity's stderr to tell a dialog that never opened from a cancel; the answer
    // itself only ever travels on stdout.
    expect(options.stdio).toEqual([
      'ignore',
      'pipe',
      process.platform === 'darwin' || process.platform === 'win32' ? 'ignore' : 'pipe',
    ]);
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

  it('on Linux reports a zenity that could not open, naming zenity and its first stderr line', async () => {
    vi.spyOn(process, 'platform', 'get').mockReturnValue('linux');
    const child = new FakeChild();
    mocks.spawn.mockReturnValue(child);
    const pending = promptNativeSecret('title', 'message');
    const [, , options] = mocks.spawn.mock.calls[0] as [string, string[], { stdio: string[] }];
    expect(options.stdio).toEqual(['ignore', 'pipe', 'pipe']);
    child.stderr.emit(
      'data',
      Buffer.from('This option is not available. Please see --help for all possible usages.\n')
    );
    child.emit('close', 255);
    await expect(pending).rejects.toThrow(
      /could not open its secure password dialog \(zenity exited with code 255: This option is not available/
    );
    expect((await pending.catch((error: Error) => error.message)) as string).not.toMatch(/cancel/i);
  });

  it('on Linux still reports a click on Cancel as a cancellation', async () => {
    vi.spyOn(process, 'platform', 'get').mockReturnValue('linux');
    const child = new FakeChild();
    mocks.spawn.mockReturnValue(child);
    const pending = promptNativeSecret('title', 'message');
    child.stderr.emit(
      'data',
      Buffer.from('Gtk-Message: GtkDialog mapped without a transient parent.\n')
    );
    child.emit('close', 1);
    await expect(pending).resolves.toBeUndefined();
  });

  it('on Linux names zenity when the helper cannot be started', async () => {
    vi.spyOn(process, 'platform', 'get').mockReturnValue('linux');
    const child = new FakeChild();
    mocks.spawn.mockReturnValue(child);
    const pending = promptNativeSecret('title', 'message');
    child.emit('error', Object.assign(new Error('spawn EACCES'), { code: 'EACCES' }));
    await expect(pending).rejects.toThrow(/could not start zenity/);
  });

  it('closes the open dialog when asked, and does nothing when no prompt is open', async () => {
    closeNativeSecretPrompt();
    const child = new FakeChild();
    mocks.spawn.mockReturnValue(child);
    const pending = promptNativeSecret('title', 'message');
    closeNativeSecretPrompt();
    expect(child.kill).toHaveBeenCalledOnce();
    child.signalCode = 'SIGTERM';
    child.emit('close', null);
    await expect(pending).resolves.toBeUndefined();
    // The prompt has finished, so a later quit has nothing left to close.
    closeNativeSecretPrompt();
    expect(child.kill).toHaveBeenCalledOnce();
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
 * DOCS-6: the brand is "Biorouter", lowercase r. The daemon prompts of 1.92.0 said "BioRouter", so
 * the manual had to quote a spelling it uses nowhere else, in a dialog it told people to trust
 * with a secret. Read at the source: the prompts are native dialogs no test renders.
 */
describe('native secret prompts spell the brand "Biorouter"', () => {
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
    // The Crew vault's passphrase and its confirmation. The shared daemon asks for no secret.
    expect(calls.length).toBeGreaterThanOrEqual(2);
    const texts = calls.flatMap(literals);
    expect(texts.length).toBeGreaterThanOrEqual(4);
    expect(texts.filter((text) => text.includes('BioRouter'))).toEqual([]);
  });

  it('in every sentence nativeSecretPrompt.ts shows', () => {
    const texts = literals(source('nativeSecretPrompt.ts'));
    expect(texts.length).toBeGreaterThan(5);
    expect(texts.filter((text) => text.includes('BioRouter'))).toEqual([]);
  });
});

describe('nativePromptOutcome', () => {
  const outcome = (
    platform: typeof process.platform,
    code: number | null,
    stderr = '',
    answer = ''
  ) => nativePromptOutcome({ platform, code, stderr, answer });

  it('returns a typed answer on every platform', () => {
    for (const platform of ['linux', 'darwin', 'win32'] as const)
      expect(outcome(platform, 0, '', 'typed')).toEqual({ kind: 'answer', answer: 'typed' });
  });

  it('keeps the old mapping on macOS and Windows: any other exit is a cancellation', () => {
    for (const platform of ['darwin', 'win32'] as const)
      for (const code of [0, 1, 255, null])
        expect(outcome(platform, code, 'cannot open display')).toEqual({ kind: 'cancelled' });
  });

  it('on Linux treats Cancel, an empty answer and a signal as no answer', () => {
    expect(outcome('linux', 1)).toEqual({ kind: 'cancelled' });
    expect(outcome('linux', 1, 'Gtk-Message: mapped without a transient parent')).toEqual({
      kind: 'cancelled',
    });
    expect(outcome('linux', 0)).toEqual({ kind: 'cancelled' });
    expect(outcome('linux', null)).toEqual({ kind: 'cancelled' });
  });

  it('on Linux reports a dialog that never opened instead of calling it cancelled', () => {
    const display = outcome(
      'linux',
      1,
      '\n(zenity:12): Gtk-WARNING **: cannot open display: :77\n'
    );
    expect(display.kind).toBe('failed');
    expect(display.kind === 'failed' && display.message).toContain(
      'zenity exited with code 1: (zenity:12): Gtk-WARNING **: cannot open display: :77'
    );
    const args = outcome('linux', 255);
    expect(args.kind === 'failed' && args.message).toContain('zenity exited with code 255)');
    const long = outcome('linux', 255, 'x'.repeat(1000));
    expect(long.kind === 'failed' && long.message.length).toBeLessThan(500);
  });
});

/**
 * 1.92.0: zenity rejected the approval prompt's text in a C locale because it wrote the range
 * "32 to 4096" with an en dash (U+2013), exited 255, and the app reported the person as having
 * cancelled. GLib cannot convert non-ASCII argv text without a UTF-8 locale, so every prompt
 * title and message stays plain ASCII.
 */
describe('native prompt text is plain ASCII', () => {
  const source = (name: string) => readFileSync(join(__dirname, name), 'utf8');

  it('in every promptNativeSecret call in main.ts', () => {
    const calls = [...source('main.ts').matchAll(/promptNativeSecret\(([\s\S]*?)\);/g)].map(
      (match) => match[1]
    );
    expect(calls.length).toBeGreaterThanOrEqual(2);
    // eslint-disable-next-line no-control-regex
    expect(calls.filter((call) => /[^\x00-\x7f]/.test(call))).toEqual([]);
  });

  it('in every sentence nativeSecretPrompt.ts passes to a dialog or error', () => {
    const code = source('nativeSecretPrompt.ts')
      .replace(/\/\*[\s\S]*?\*\//g, '')
      .replace(/^\s*\/\/.*$/gm, '');
    // eslint-disable-next-line no-control-regex
    expect(code.match(/[^\x00-\x7f]/g) ?? []).toEqual([]);
  });
});
