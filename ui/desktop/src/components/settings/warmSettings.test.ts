import { beforeEach, describe, expect, it, vi } from 'vitest';
import { recall, remember } from './lastKnown';
import { warmSettings } from './warmSettings';

/**
 * Settings asks for what the Chat and App tabs show while it opens, so the first visit to either
 * has an answer to start from (vision round 8, V4-R7-2).
 */
const flush = () => new Promise<void>((resolve) => setTimeout(resolve, 0));

function bridge(platform = 'darwin') {
  Object.assign(window, {
    electron: {
      platform,
      getMenuBarIconState: vi.fn().mockResolvedValue(false),
      getDockIconState: vi.fn().mockResolvedValue(true),
      getWakelockState: vi.fn().mockResolvedValue(true),
      getSpellcheckState: vi.fn().mockResolvedValue(false),
    },
  });
  return window.electron as unknown as Record<string, ReturnType<typeof vi.fn>>;
}

const configValues: Record<string, unknown> = {
  BIOROUTER_MODE: 'approve',
  BIOROUTER_MAX_TURNS: '25',
  WORKSPACE_ANNOUNCE_ONLY: true,
};

describe('warmSettings', () => {
  const read = vi.fn(async (key: string, _isSecret: boolean) => configValues[key]);

  beforeEach(() => {
    read.mockReset();
    read.mockImplementation(async (key: string) => configValues[key]);
    bridge();
  });

  it('reads every Chat and App setting and keeps each answer the way the tabs would read it', async () => {
    warmSettings(read);
    await flush();
    expect(recall('approvalMode')).toBe('approve');
    expect(recall('maxTurns')).toBe(25);
    expect(recall('announceOnly')).toBe(true);
    expect(recall('menuBarIcon')).toBe(false);
    expect(recall('dockIcon')).toBe(true);
    expect(recall('wakelock')).toBe(true);
    expect(recall('spellcheck')).toBe(false);
  });

  it('reads only the three config keys, none of them secret', async () => {
    warmSettings(read);
    await flush();
    expect(read.mock.calls.map(([key]) => key).sort()).toEqual([
      'BIOROUTER_MAX_TURNS',
      'BIOROUTER_MODE',
      'WORKSPACE_ANNOUNCE_ONLY',
    ]);
    expect(read.mock.calls.every(([, isSecret]) => isSecret === false)).toBe(true);
  });

  it('keeps "nothing saved" the way the tab does: Autonomous, and no Max turns', async () => {
    read.mockImplementation(async () => undefined);
    warmSettings(read);
    await flush();
    expect(recall('approvalMode')).toBe('auto');
    expect(recall('maxTurns')).toBeNull();
    expect(recall('announceOnly')).toBe(false);
  });

  it('does not read a Dock state off macOS, where the App tab has no Dock row', async () => {
    const electron = bridge('linux');
    warmSettings(read);
    await flush();
    expect(electron.getDockIconState).not.toHaveBeenCalled();
    expect(recall('dockIcon')).toBeUndefined();
  });

  it('does not ask again for a setting something already answered', async () => {
    const electron = bridge();
    remember('wakelock', false);
    remember('approvalMode', 'chat');
    warmSettings(read);
    await flush();
    expect(electron.getWakelockState).not.toHaveBeenCalled();
    expect(read).not.toHaveBeenCalledWith('BIOROUTER_MODE', false);
    expect(recall('wakelock')).toBe(false);
    expect(recall('approvalMode')).toBe('chat');
  });

  it('drops a failed read and leaves the setting unknown, for the tab to read again', async () => {
    const electron = bridge();
    electron.getWakelockState.mockRejectedValue(new Error('no main process'));
    read.mockImplementation(async (key: string) => {
      if (key === 'BIOROUTER_MODE') throw new Error('daemon down');
      return configValues[key];
    });
    warmSettings(read);
    await flush();
    expect(recall('wakelock')).toBeUndefined();
    expect(recall('approvalMode')).toBeUndefined();
    expect(recall('maxTurns')).toBe(25);
  });

  it('survives a main-process bridge that is not there at all', async () => {
    Object.assign(window, { electron: undefined });
    expect(() => warmSettings(read)).not.toThrow();
    await flush();
    expect(recall('maxTurns')).toBe(25);
    expect(recall('menuBarIcon')).toBeUndefined();
  });

  it('lets a change made while a read was out win over its late answer', async () => {
    let answer: (value: boolean) => void = () => {};
    const electron = bridge();
    electron.getWakelockState.mockImplementation(
      () => new Promise<boolean>((resolve) => (answer = resolve))
    );
    warmSettings(read);
    await flush();
    remember('wakelock', false); // the person flipped it
    answer(true); // the stale answer lands
    await flush();
    expect(recall('wakelock')).toBe(false);
  });
});
