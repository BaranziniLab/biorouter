// @vitest-environment node
import { describe, expect, it, vi } from 'vitest';
import fs from 'node:fs/promises';
import path from 'node:path';
import { attachedDaemonKey } from './attachedDaemonKey';

describe('authentication for an attached window', () => {
  const daemons = new Map([
    [1, { secretKey: 'first-secret', userActionKey: 'first-proof' }],
    [2, { secretKey: 'second-secret', userActionKey: 'second-proof' }],
    [3, { secretKey: '', userActionKey: '' }],
  ]);

  it.each(['secretKey', 'userActionKey'] as const)(
    'keeps %s on the owning daemon after settings change',
    (key) => {
      const readSettings = vi.fn(() => 'new-external-daemon-key');
      expect(attachedDaemonKey(daemons, 1, key, readSettings)).toBe(daemons.get(1)![key]);
      expect(attachedDaemonKey(daemons, 2, key, readSettings)).toBe(daemons.get(2)![key]);
      expect(readSettings).not.toHaveBeenCalled();
    }
  );

  it('keeps an empty attached key empty rather than sending another daemon key', () => {
    const readSettings = vi.fn(() => 'unrelated-proof');
    expect(attachedDaemonKey(daemons, 3, 'userActionKey', readSettings)).toBe('');
    expect(readSettings).not.toHaveBeenCalled();
  });

  it.each([undefined, 4])('reads settings before window %s has attached', (windowId) => {
    const readSettings = vi.fn(() => 'startup-key');
    expect(attachedDaemonKey(daemons, windowId, 'secretKey', readSettings)).toBe('startup-key');
    expect(readSettings).toHaveBeenCalledOnce();
  });

  it('derives the window identity from the IPC sender for both getters', async () => {
    const main = await fs.readFile(path.join(__dirname, '..', 'main.ts'), 'utf8');
    for (const [channel, key] of [
      ['get-secret-key', 'secretKey'],
      ['get-user-action-key', 'userActionKey'],
    ]) {
      const start = main.indexOf(`ipcMain.handle('${channel}'`);
      expect(start).toBeGreaterThan(-1);
      const handler = main.slice(start, main.indexOf('\n});', start));
      expect(handler).toContain('BrowserWindow.fromWebContents(event.sender)?.id');
      expect(handler).toContain(`attachedDaemonKey(windowDaemons, windowId, '${key}'`);
    }
  });
});
