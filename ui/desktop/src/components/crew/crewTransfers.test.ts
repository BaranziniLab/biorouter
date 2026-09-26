import { beforeEach, describe, expect, it, vi } from 'vitest';
import { beginTransfer } from './crewTransfers';

const mocks = vi.hoisted(() => ({
  crewHttp: vi.fn(),
}));

vi.mock('./crewApi', async () => {
  const actual = await vi.importActual<typeof import('./crewApi')>('./crewApi');
  return { ...actual, crewHttp: mocks.crewHttp };
});

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

describe('Crew transfer privacy handoff', () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    mocks.crewHttp.mockReset();
    Object.defineProperty(window, 'electron', {
      configurable: true,
      writable: true,
      value: { crewSelectTransferFile: vi.fn() },
    });
  });

  it('captures the verified mode before an async picker and starts only with its opaque capability', async () => {
    const selection = deferred<{ capability_id: string; name: string }>();
    const picker = window.electron.crewSelectTransferFile as ReturnType<typeof vi.fn>;
    picker.mockReturnValue(selection.promise);
    mocks.crewHttp.mockResolvedValue({
      id: 'transfer-1',
      connection_id: 'connection-1',
      channel_id: 'channel-1',
      direction: 'upload',
    });

    const transfer = beginTransfer({
      expected_mode: 'private',
      connection_id: 'connection-1',
      channel_id: 'channel-1',
      direction: 'upload',
    });

    expect(picker).toHaveBeenCalledWith(
      expect.objectContaining({
        expectedMode: 'private',
        connectionId: 'connection-1',
        channelId: 'channel-1',
      })
    );
    selection.resolve({ capability_id: 'capability-1', name: 'notes.txt' });
    await expect(transfer).resolves.toMatchObject({ id: 'transfer-1' });
    expect(mocks.crewHttp).toHaveBeenCalledWith(
      '/transfers',
      'POST',
      expect.objectContaining({
        connection_id: 'connection-1',
        channel_id: 'channel-1',
        file_capability: 'capability-1',
      })
    );
    expect(mocks.crewHttp.mock.calls[0][2]).not.toHaveProperty('expected_mode');
  });

  it('does not turn a cancelled async picker into a transfer request', async () => {
    const picker = window.electron.crewSelectTransferFile as ReturnType<typeof vi.fn>;
    picker.mockResolvedValue(null);

    await expect(
      beginTransfer({
        expected_mode: 'public',
        connection_id: 'connection-1',
        channel_id: 'channel-1',
        direction: 'upload',
      })
    ).resolves.toBeNull();
    expect(mocks.crewHttp).not.toHaveBeenCalled();
  });
});
