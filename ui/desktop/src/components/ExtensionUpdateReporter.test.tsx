import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render } from '@testing-library/react';
import type { ExtensionUpdateEvent } from '../utils/extensionUpdater';

const toastError = vi.fn();
const success = vi.fn();
vi.mock('../toasts', () => ({
  toastError: (...a: unknown[]) => toastError(...a),
  toastService: { success: (...a: unknown[]) => success(...a) },
}));

import ExtensionUpdateReporter from './ExtensionUpdateReporter';

let emit: (e: ExtensionUpdateEvent) => void = () => {};

beforeEach(() => {
  toastError.mockReset();
  success.mockReset();
  // @ts-expect-error — partial stub, only what the reporter touches.
  window.electron = {
    onExtensionUpdateEvent: (cb: (e: ExtensionUpdateEvent) => void) => {
      emit = cb;
      // Matches the bridge's disposer shape, and it really is called: the
      // shared setup installs an `afterEach(cleanup)` (src/test/setup.ts), so
      // React unmounts the reporter after every test here. Clearing `emit` on
      // dispose keeps a stale callback from a previous test out of the next
      // one. Counting listeners across mounts is the other file's job,
      // ExtensionUpdateReporter.listeners.test.tsx.
      return () => {
        emit = () => {};
      };
    },
  };
});

describe('ExtensionUpdateReporter', () => {
  it('reports a failed update that used to be silent', () => {
    render(<ExtensionUpdateReporter />);
    emit({
      type: 'update-error',
      ext: 'spokeagent',
      displayName: 'SPOKE Agent',
      error: 'uv sync failed',
    });

    expect(toastError).toHaveBeenCalledTimes(1);
    const arg = toastError.mock.calls[0][0];
    expect(arg.title).toContain('SPOKE Agent');
    expect(arg.msg).toContain('uv sync failed');
  });

  it('offers the debug session on a failure', () => {
    render(<ExtensionUpdateReporter />);
    emit({ type: 'update-error', ext: 'spokeagent', displayName: 'SPOKE Agent', error: 'boom' });

    const arg = toastError.mock.calls[0][0];
    expect(arg.debugFailure).toMatchObject({ kind: 'extension', name: 'spokeagent' });
  });

  it('names the extension even when only the id is known', () => {
    render(<ExtensionUpdateReporter />);
    emit({ type: 'update-error', ext: 'cdwagent', error: 'boom' });
    expect(toastError.mock.calls[0][0].title).toContain('cdwagent');
  });

  it('stays quiet when nothing was updated', () => {
    render(<ExtensionUpdateReporter />);
    emit({ type: 'all-done', updatedCount: 0 });
    expect(success).not.toHaveBeenCalled();
  });

  it('reports a successful update once, with correct plurality', () => {
    render(<ExtensionUpdateReporter />);
    emit({ type: 'all-done', updatedCount: 1 });
    expect(success).toHaveBeenCalledTimes(1);
    expect(success.mock.calls[0][0].msg).toContain('1 extension updated');

    emit({ type: 'all-done', updatedCount: 3 });
    expect(success.mock.calls[1][0].msg).toContain('3 extensions updated');
  });

  it('ignores the progress chatter', () => {
    render(<ExtensionUpdateReporter />);
    emit({ type: 'update-found', ext: 'x' });
    emit({ type: 'update-start', ext: 'x' });
    emit({ type: 'update-progress', ext: 'x', percent: 50 });
    emit({ type: 'update-done', ext: 'x' });
    expect(toastError).not.toHaveBeenCalled();
    expect(success).not.toHaveBeenCalled();
  });
});
