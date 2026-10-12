import { beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('../api', () => ({
  updateSessionName: vi.fn(async () => ({ data: {} })),
}));

vi.mock('./userAction', () => ({
  userActionHeaders: async () => ({ 'X-User-Action': 'test-proof' }),
}));

vi.mock('../toasts', () => ({
  toastError: vi.fn(),
}));

import { updateSessionName } from '../api';
import { toastError } from '../toasts';
import {
  normalizeSessionName,
  RENAME_FAILED_TOAST_TITLE,
  renameSessionOptimistically,
  SESSION_NAME_MAX_LENGTH,
  subscribeSessionNameChanges,
  type SessionNameChange,
} from './sessionNameSync';

const previous = { name: 'Old name', userSetName: false };

function recordChanges() {
  const changes: SessionNameChange[] = [];
  const unsubscribe = subscribeSessionNameChanges((change) => changes.push(change));
  return { changes, unsubscribe };
}

describe('renameSessionOptimistically', () => {
  beforeEach(() => {
    vi.mocked(updateSessionName).mockReset();
    vi.mocked(updateSessionName).mockResolvedValue({ data: {} } as never);
    vi.mocked(toastError).mockClear();
  });

  it('announces the new name first, then saves it with the user proof', async () => {
    const { changes, unsubscribe } = recordChanges();
    const outcome = await renameSessionOptimistically('s1', '  New name  ', previous);
    unsubscribe();

    expect(outcome).toBe('renamed');
    expect(changes[0]).toEqual({
      sessionId: 's1',
      name: 'New name',
      userSetName: true,
      origin: 'user',
    });
    expect(updateSessionName).toHaveBeenCalledWith({
      path: { session_id: 's1' },
      body: { name: 'New name' },
      headers: { 'X-User-Action': 'test-proof' },
      throwOnError: true,
    });
    expect(toastError).not.toHaveBeenCalled();
  });

  it('makes no call for an empty or unchanged name', async () => {
    expect(await renameSessionOptimistically('s1', '   ', previous)).toBe('unchanged');
    expect(await renameSessionOptimistically('s1', ' Old name ', previous)).toBe('unchanged');
    expect(updateSessionName).not.toHaveBeenCalled();
  });

  it('rolls back to the previous name and shows the daemon sentence on a refusal', async () => {
    const refusal = 'This chat is private. Rename it from a Biorouter window.';
    vi.mocked(updateSessionName).mockRejectedValueOnce(refusal);
    const { changes, unsubscribe } = recordChanges();

    const outcome = await renameSessionOptimistically('s1', 'New name', previous);
    unsubscribe();

    expect(outcome).toBe('failed');
    expect(changes.map((change) => [change.name, change.origin])).toEqual([
      ['New name', 'user'],
      ['Old name', 'sync'],
    ]);
    expect(toastError).toHaveBeenCalledWith({ title: RENAME_FAILED_TOAST_TITLE, msg: refusal });
  });

  it('uses the error message when the failure is an Error', async () => {
    vi.mocked(updateSessionName).mockRejectedValueOnce(new Error('http 500'));
    await renameSessionOptimistically('s1', 'New name', previous);
    expect(toastError).toHaveBeenCalledWith({ title: RENAME_FAILED_TOAST_TITLE, msg: 'http 500' });
  });
});

describe('normalizeSessionName', () => {
  it('turns pasted newlines into spaces and trims', () => {
    expect(normalizeSessionName('  one\ntwo\r\nthree  ')).toBe('one two three');
  });

  it('caps the name at the server limit', () => {
    expect(normalizeSessionName('x'.repeat(SESSION_NAME_MAX_LENGTH + 20))).toHaveLength(
      SESSION_NAME_MAX_LENGTH
    );
  });
});
