import { afterEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({
  appendFileSync: vi.fn(),
  getFile: vi.fn(() => ({ path: '/tmp/biorouter-test-main.log' })),
  startCatching: vi.fn(),
}));

vi.mock('electron', () => ({
  app: {
    getPath: () => '/tmp/biorouter-test-user-data',
    isPackaged: false,
  },
}));

vi.mock('electron-log', () => ({
  default: {
    transports: {
      file: {
        getFile: mocks.getFile,
        resolvePathFn: undefined,
        level: undefined,
        sync: true,
      },
      console: { level: undefined },
    },
    errorHandler: { startCatching: mocks.startCatching },
  },
}));

vi.mock('node:fs', async (importOriginal) => {
  const actual = await importOriginal<typeof import('node:fs')>();
  const mocked = { ...actual, appendFileSync: mocks.appendFileSync };
  return { ...mocked, default: mocked };
});

import log, { logStartupFailure } from './logger';

afterEach(() => {
  vi.restoreAllMocks();
  mocks.appendFileSync.mockReset();
  mocks.getFile.mockClear();
  mocks.startCatching.mockClear();
});

describe('logStartupFailure', () => {
  it('writes one synchronous fatal record to the configured logger path', () => {
    const error = new Error('window creation failed');

    logStartupFailure(error);

    expect(mocks.getFile).toHaveBeenCalledOnce();
    expect(mocks.appendFileSync).toHaveBeenCalledOnce();
    expect(mocks.appendFileSync).toHaveBeenCalledWith(
      '/tmp/biorouter-test-main.log',
      expect.stringContaining('[Main] Fatal error during startup: Error: window creation failed'),
      'utf8'
    );
    expect(mocks.appendFileSync.mock.calls[0][1]).toContain(error.stack);
    expect(mocks.appendFileSync.mock.calls[0][1]).toMatch(/\n$/);
  });

  it('swallows path and write failures so startup can still show its dialog', () => {
    const error = new Error('startup failed');
    mocks.getFile.mockImplementation(() => {
      throw new Error('path unavailable');
    });

    expect(() => logStartupFailure(error)).not.toThrow();
    expect(mocks.appendFileSync).not.toHaveBeenCalled();

    mocks.getFile.mockImplementation(() => ({ path: '/tmp/biorouter-test-main.log' }));
    mocks.appendFileSync.mockImplementation(() => {
      throw new Error('disk full');
    });

    expect(() => logStartupFailure(error)).not.toThrow();
  });

  it('keeps ordinary logger writes asynchronous', () => {
    expect(log.transports.file.sync).toBe(false);
  });
});
