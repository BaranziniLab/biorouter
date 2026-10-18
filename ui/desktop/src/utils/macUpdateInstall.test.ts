// @vitest-environment node

/**
 * "Restart & update" did nothing on some Macs. Each case here is a way it
 * could: clicked while Squirrel.Mac was still staging, Squirrel failing after
 * the download, or the quit itself stalling with every window closed.
 */
import { describe, it, expect, vi } from 'vitest';
import { MacInstallCoordinator } from './macUpdateInstall';

function setup(overrides: { quitAndInstall?: () => void } = {}) {
  const events: Array<{ event: string; data?: unknown }> = [];
  const timers: Array<{ callback: () => void; ms: number; cleared: boolean }> = [];
  const quitAndInstall = vi.fn(overrides.quitAndInstall ?? (() => {}));
  const ensureWindow = vi.fn();
  const coordinator = new MacInstallCoordinator({
    quitAndInstall,
    emit: (event, data) => events.push({ event, data }),
    ensureWindow,
    log: () => {},
    setTimer: (callback, ms) => {
      const timer = { callback, ms, cleared: false };
      timers.push(timer);
      return timer;
    },
    clearTimer: (timer) => {
      (timer as { cleared: boolean }).cleared = true;
    },
    stagingTimeoutMs: 1000,
    quitTimeoutMs: 500,
  });
  const fireLiveTimer = () => {
    const live = timers.filter((t) => !t.cleared);
    expect(live).toHaveLength(1);
    live[0].cleared = true;
    live[0].callback();
  };
  const names = () => events.map((e) => e.event);
  return { coordinator, events, names, quitAndInstall, ensureWindow, fireLiveTimer, timers };
}

describe('MacInstallCoordinator', () => {
  it('says "preparing", not "ready", when electron-updater finishes the download', () => {
    const { coordinator, names } = setup();
    coordinator.downloaded('1.93.2');
    expect(names()).toEqual(['update-preparing']);
    expect(coordinator.currentStage).toBe('staging');
    expect(coordinator.ownsErrors).toBe(true);
  });

  it('says "ready" once Squirrel.Mac has staged it', () => {
    const { coordinator, events } = setup();
    coordinator.downloaded('1.93.2');
    expect(coordinator.staged()).toBe(true);
    expect(events[1]).toEqual({ event: 'update-downloaded', data: { version: '1.93.2' } });
    expect(coordinator.ownsErrors).toBe(false);
  });

  it('installs at once when the update is staged', () => {
    const { coordinator, names, quitAndInstall } = setup();
    coordinator.downloaded('1.93.2');
    coordinator.staged();
    expect(coordinator.requestInstall()).toEqual({ ok: true });
    expect(quitAndInstall).toHaveBeenCalledTimes(1);
    expect(names()).toContain('update-installing');
  });

  it('a click while Squirrel.Mac is staging installs the moment it is done', () => {
    const { coordinator, names, quitAndInstall } = setup();
    coordinator.downloaded('1.93.2');
    expect(coordinator.requestInstall()).toEqual({ ok: true });
    // Visible at once, without quitting into a half-staged update.
    expect(names()).toEqual(['update-preparing', 'update-installing']);
    expect(quitAndInstall).not.toHaveBeenCalled();

    // No "ready" notification for an update that is already installing.
    expect(coordinator.staged()).toBe(false);
    expect(quitAndInstall).toHaveBeenCalledTimes(1);
  });

  it('turns a Squirrel.Mac failure into a reason, and refuses to install after it', () => {
    const { coordinator, events } = setup();
    coordinator.downloaded('1.93.2');
    coordinator.fail('Cannot update while running on a read-only volume.');
    expect(events[events.length - 1]).toEqual({
      event: 'install-failed',
      data: { version: '1.93.2', message: 'Cannot update while running on a read-only volume.' },
    });
    expect(coordinator.requestInstall().ok).toBe(false);
    expect(coordinator.pendingVersion).toBeNull();
  });

  it('gives up on a staging that never finishes', () => {
    const { coordinator, names, fireLiveTimer } = setup();
    coordinator.downloaded('1.93.2');
    fireLiveTimer();
    expect(names()).toEqual(['update-preparing', 'install-failed']);
  });

  it('still accepts a staging that finished after its timeout', () => {
    const { coordinator, fireLiveTimer } = setup();
    coordinator.downloaded('1.93.2');
    fireLiveTimer();
    expect(coordinator.staged()).toBe(true);
    expect(coordinator.requestInstall()).toEqual({ ok: true });
  });

  it('reports a quit that stalls, in a window it opens if the install closed them all', () => {
    const { coordinator, names, ensureWindow, fireLiveTimer } = setup();
    coordinator.downloaded('1.93.2');
    coordinator.staged();
    coordinator.requestInstall();
    fireLiveTimer();
    expect(names()[names().length - 1]).toBe('install-failed');
    expect(ensureWindow).toHaveBeenCalledTimes(1);
  });

  it('reports a quitAndInstall that throws', () => {
    const { coordinator, names } = setup({
      quitAndInstall: () => {
        throw new Error("No update available, can't quit and install");
      },
    });
    coordinator.downloaded('1.93.2');
    coordinator.staged();
    coordinator.requestInstall();
    expect(names()[names().length - 1]).toBe('install-failed');
  });

  it('ignores a repeated download of the version it already has', () => {
    const { coordinator, names } = setup();
    coordinator.downloaded('1.93.2');
    coordinator.staged();
    coordinator.downloaded('1.93.2');
    expect(names()).toEqual(['update-preparing', 'update-downloaded']);
    expect(coordinator.currentStage).toBe('staged');
  });

  it('stages a newer version that arrives after one was ready', () => {
    const { coordinator, events } = setup();
    coordinator.downloaded('1.93.2');
    coordinator.staged();
    coordinator.downloaded('1.93.3');
    expect(events[events.length - 1]).toEqual({
      event: 'update-preparing',
      data: { version: '1.93.3' },
    });
    expect(coordinator.pendingVersion).toBe('1.93.3');
  });

  it('ignores a staging it never asked for', () => {
    const { coordinator, names } = setup();
    expect(coordinator.staged()).toBe(false);
    expect(names()).toEqual([]);
  });

  it('cancels its timers when the stage moves on', () => {
    const { coordinator, timers } = setup();
    coordinator.downloaded('1.93.2');
    coordinator.staged();
    expect(timers.every((t) => t.cleared)).toBe(true);
  });
});
