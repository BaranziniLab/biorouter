import { Readable } from 'node:stream';
import { describe, expect, it, vi } from 'vitest';

import {
  createDevelopmentApprovalReader,
  DEVELOPMENT_APPROVAL_STDIN_FLAG,
} from './developmentApprovalInput';

function input() {
  return new Readable({ read() {} });
}

function options(overrides: Partial<Parameters<typeof createDevelopmentApprovalReader>[0]> = {}) {
  return {
    args: [DEVELOPMENT_APPROVAL_STDIN_FLAG],
    isPackaged: false,
    developmentProfileRoot: '/private/tmp/profile',
    testDriverEnabled: true,
    sharedDaemonEnabled: true,
    input: input(),
    inputIsPipe: () => true,
    validate: (secret: string) => {
      if (secret.length < 32 || secret.length > 4096 || /[^!-~]/.test(secret))
        throw new Error('invalid');
    },
    ...overrides,
  };
}

describe('development approval stdin', () => {
  it.each([
    ['packaged app', { isPackaged: true }],
    ['missing profile', { developmentProfileRoot: undefined }],
    ['test driver disabled', { testDriverEnabled: false }],
    ['shared daemon disabled', { sharedDaemonEnabled: false }],
  ])('rejects eligibility failure: %s', (_name, override) => {
    expect(() => createDevelopmentApprovalReader(options(override))).toThrow(
      /Approval stdin requires/
    );
  });

  it('rejects a terminal input and duplicate flags', () => {
    expect(() => createDevelopmentApprovalReader(options({ inputIsPipe: () => false }))).toThrow(
      /inherited pipe/
    );
    expect(() =>
      createDevelopmentApprovalReader(
        options({ args: [DEVELOPMENT_APPROVAL_STDIN_FLAG, DEVELOPMENT_APPROVAL_STDIN_FLAG] })
      )
    ).toThrow(/once/);
  });

  it.each(['\n', '\r\n'])('reads one bounded secret with %j terminator', async (terminator) => {
    const stream = input();
    const reader = createDevelopmentApprovalReader(options({ input: stream }))!;
    const promise = reader();
    stream.push(Buffer.from('a'.repeat(32) + terminator));
    stream.push(null);
    await expect(promise).resolves.toBe('a'.repeat(32));
    await expect(reader()).rejects.toThrow(/already consumed/);
  });

  it.each(['\n\n', '\r\r', '\r\n\r\n'])(
    'rejects repeated line terminators: %j',
    async (terminators) => {
      const stream = input();
      const reader = createDevelopmentApprovalReader(options({ input: stream }))!;
      const promise = reader().then(
        () => null,
        (error: Error) => error
      );
      stream.push(Buffer.from('a'.repeat(32) + terminators));
      stream.push(null);
      await expect(promise).resolves.toMatchObject({
        message: expect.stringMatching(/invalid/),
      });
    }
  );

  it('rejects oversized, invalid, and closed input without echoing it', async () => {
    const oversized = input();
    const oversizedReader = createDevelopmentApprovalReader(options({ input: oversized }))!;
    const oversizedPromise = oversizedReader();
    const oversizedRejection = oversizedPromise.then(
      () => null,
      (error: Error) => error
    );
    oversized.push(Buffer.alloc(4099, 65));
    await expect(oversizedRejection).resolves.toMatchObject({
      message: expect.stringMatching(/size limit/),
    });

    const invalid = input();
    const invalidReader = createDevelopmentApprovalReader(options({ input: invalid }))!;
    const invalidPromise = invalidReader();
    const invalidRejection = invalidPromise.then(
      () => null,
      (error: Error) => error
    );
    invalid.push(Buffer.from('too-short\n'));
    invalid.push(null);
    await expect(invalidRejection).resolves.toMatchObject({
      message: expect.stringMatching(/invalid/),
    });

    const closed = input();
    const closedReader = createDevelopmentApprovalReader(options({ input: closed }))!;
    const closedPromise = closedReader();
    const closedRejection = closedPromise.then(
      () => null,
      (error: Error) => error
    );
    closed.destroy();
    await expect(closedRejection).resolves.toMatchObject({
      message: expect.stringMatching(/closed/),
    });
  });

  it('times out without accepting partial input', async () => {
    vi.useFakeTimers();
    try {
      const stream = input();
      const reader = createDevelopmentApprovalReader(options({ input: stream }))!;
      const promise = reader();
      const rejection = promise.then(
        () => null,
        (error: Error) => error
      );
      stream.push(Buffer.from('a'.repeat(32)));
      await vi.advanceTimersByTimeAsync(30_000);
      await expect(rejection).resolves.toMatchObject({
        message: expect.stringMatching(/timed out/),
      });
    } finally {
      vi.useRealTimers();
    }
  });
});
