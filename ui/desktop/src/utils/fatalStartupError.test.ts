// @vitest-environment node
import { describe, expect, it, vi } from 'vitest';
import { reportFatalStartupError, type FatalStartupDeps } from './fatalStartupError';

function deps(overrides: Partial<FatalStartupDeps> = {}) {
  const calls: string[] = [];
  const base: FatalStartupDeps = {
    platform: 'linux',
    logStartupFailure: vi.fn(() => void calls.push('log')),
    showErrorBox: vi.fn(() => void calls.push('showErrorBox')),
    showMessageBox: vi.fn(async () => {
      calls.push('showMessageBox');
      return { response: 0 };
    }),
    quit: vi.fn(() => void calls.push('quit')),
    isQuitting: () => false,
    ...overrides,
  };
  return { deps: base, calls };
}

describe('reportFatalStartupError', () => {
  it('on Linux never uses the synchronous error box, and quits once the dialog closes', async () => {
    const { deps: d, calls } = deps();
    await reportFatalStartupError(new Error('boom'), d);
    expect(d.showErrorBox).not.toHaveBeenCalled();
    expect(d.showMessageBox).toHaveBeenCalledWith(
      expect.objectContaining({
        type: 'error',
        title: 'Biorouter Error',
        message: 'Failed to create main window: Error: boom',
      })
    );
    expect(calls).toEqual(['log', 'showMessageBox', 'quit']);
  });

  it('on Linux returns before the dialog closes, so the main loop keeps running', async () => {
    let close!: () => void;
    const { deps: d } = deps({
      showMessageBox: vi.fn(() => new Promise<unknown>((resolve) => (close = () => resolve({})))),
    });
    const done = reportFatalStartupError(new Error('boom'), d);
    await Promise.resolve();
    expect(d.quit).not.toHaveBeenCalled();
    close();
    await done;
    expect(d.quit).toHaveBeenCalledOnce();
  });

  it('on Linux still quits when the dialog itself fails', async () => {
    const { deps: d } = deps({ showMessageBox: vi.fn(async () => Promise.reject(new Error('x'))) });
    await reportFatalStartupError(new Error('boom'), d);
    expect(d.quit).toHaveBeenCalledOnce();
  });

  it.each(['darwin', 'win32'] as const)(
    'on %s keeps the synchronous error box, logged first, then quits',
    async (platform) => {
      const { deps: d, calls } = deps({ platform });
      await reportFatalStartupError(new Error('boom'), d);
      expect(d.showMessageBox).not.toHaveBeenCalled();
      expect(d.showErrorBox).toHaveBeenCalledWith(
        'Biorouter Error',
        'Failed to create main window: Error: boom'
      );
      expect(calls).toEqual(['log', 'showErrorBox', 'quit']);
    }
  );

  it.each(['linux', 'darwin', 'win32'] as const)(
    'on %s shows no dialog when the app is already quitting, but still logs and quits',
    async (platform) => {
      const { deps: d, calls } = deps({ platform, isQuitting: () => true });
      await reportFatalStartupError(new Error('Shared daemon startup cancelled.'), d);
      expect(calls).toEqual(['log', 'quit']);
    }
  );
});
