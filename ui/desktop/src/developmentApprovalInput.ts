import type { Readable } from 'node:stream';

export const DEVELOPMENT_APPROVAL_STDIN_FLAG = '--dev-approval-key-stdin';

interface DevelopmentApprovalInputOptions {
  args: string[];
  isPackaged: boolean;
  developmentProfileRoot: string | undefined;
  testDriverEnabled: boolean;
  sharedDaemonEnabled: boolean;
  input: Readable;
  inputIsPipe: () => boolean;
  validate: (secret: string) => void;
}

export function createDevelopmentApprovalReader(
  options: DevelopmentApprovalInputOptions
): (() => Promise<string>) | undefined {
  const flags = options.args.filter((arg) => arg.startsWith(DEVELOPMENT_APPROVAL_STDIN_FLAG));
  if (flags.length === 0) return undefined;
  if (flags.length !== 1 || flags[0] !== DEVELOPMENT_APPROVAL_STDIN_FLAG)
    throw new Error('Use --dev-approval-key-stdin once, without a value.');
  if (
    options.isPackaged ||
    !options.developmentProfileRoot ||
    !options.testDriverEnabled ||
    !options.sharedDaemonEnabled
  )
    throw new Error(
      'Approval stdin requires an unpackaged app, a validated development profile, ENABLE_PLAYWRIGHT, and shared daemon mode.'
    );
  if (!options.inputIsPipe())
    throw new Error(
      'Development approval input must be an inherited pipe, not a terminal or file.'
    );

  let consumed = false;
  return async () => {
    if (consumed)
      throw new Error('Development approval input was already consumed. Relaunch explicitly.');
    consumed = true;
    return new Promise<string>((resolve, reject) => {
      const bytes = Buffer.alloc(4098);
      let size = 0;
      let settled = false;
      const input = options.input;
      const finish = (error?: Error, secret?: string) => {
        if (settled) return;
        settled = true;
        clearTimeout(timer);
        input.removeListener('data', onData);
        input.removeListener('end', onEnd);
        input.removeListener('error', onError);
        input.removeListener('close', onClose);
        input.destroy();
        bytes.fill(0);
        if (error) reject(error);
        else resolve(secret!);
      };
      const onData = (chunk: Buffer) => {
        if (!Buffer.isBuffer(chunk)) {
          finish(new Error('Development approval input must be a binary pipe.'));
          return;
        }
        if (size + chunk.length > bytes.length) {
          chunk.fill(0);
          finish(new Error('Development approval input exceeds its size limit.'));
          return;
        }
        chunk.copy(bytes, size);
        size += chunk.length;
        chunk.fill(0);
      };
      const onEnd = () => {
        let length = size;
        if (length && bytes[length - 1] === 10) length--;
        if (length && bytes[length - 1] === 13) length--;
        const secret = bytes.subarray(0, length).toString('utf8');
        try {
          options.validate(secret);
          finish(undefined, secret);
        } catch {
          finish(
            new Error('Development approval input is invalid; supply one valid approval secret.')
          );
        }
      };
      const onError = () => finish(new Error('Development approval input could not be read.'));
      const onClose = () =>
        finish(new Error('Development approval input closed before completion.'));
      const timer = setTimeout(
        () => finish(new Error('Development approval input timed out after 30 seconds.')),
        30000
      );
      input.on('data', onData);
      input.once('end', onEnd);
      input.once('error', onError);
      input.once('close', onClose);
      if (input.destroyed || input.readableEnded) onClose();
    });
  };
}
