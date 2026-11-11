import fs from 'node:fs';
import path from 'node:path';
import { describe, expect, it, vi } from 'vitest';
import { sameAnywhereStart } from './dictationAnywhere';

/**
 * `main.ts` with each run of white space as one space and none just inside a parenthesis, so
 * these literals pin what the code says and not where Prettier breaks its lines.
 */
const source = fs
  .readFileSync(path.join(__dirname, '../main.ts'), 'utf8')
  .replace(/\s+/g, ' ')
  .replace(/\( /g, '(')
  .replace(/ \)/g, ')');

describe('external hidden capture integration', () => {
  it('binds hidden capture to the same external backend and identifies its host', () => {
    const daemon = source.slice(
      source.indexOf('async function anywhereDaemon()'),
      source.indexOf('type AnywhereBackendIdentity =')
    );
    expect(daemon).toContain('if (daemon && client) return { daemon, client, owner }');
    expect(daemon).not.toContain('if (settings.externalBiorouterd?.enabled) return null');
    expect(daemon).not.toContain('if (!result.managed) return null');
    expect(source).toContain('BIOROUTER_LOCAL_BACKEND: daemon.result.managed');
  });

  it('confirms remote disclosure before authenticated status and hidden window creation', () => {
    const start = source.slice(
      source.indexOf('async function startAnywhere('),
      source.indexOf('function cancelAnywhere()')
    );
    const consent = start.indexOf('await confirmAnywhereRemote(backend, current)');
    expect(consent).toBeGreaterThan(0);
    expect(consent).toBeLessThan(start.indexOf('await anywhereAvailability(backend)'));
    expect(consent).toBeLessThan(start.indexOf("createAnywhereWindow('dictation-capture')"));
    expect(consent).toBeLessThan(start.indexOf('sendAnywhereStartIfReady()'));
    const availability = source.slice(
      source.indexOf('async function anywhereAvailability('),
      source.indexOf('async function createAnywhereWindow(')
    );
    expect(availability).toContain('X-Secret-Key');
    expect(availability).toContain('X-User-Action');
    expect(start).toContain('sameAnywhereBackend(backend)');
    expect(source).toContain("dictationMessage('MSG-REMOTE-HTTP'");
    expect(source).toContain("dictationMessage('MSG-DISCLOSE-REMOTE'");
    expect(source).toContain("crypto.createHash('sha256').update(backend.url).digest('hex')");
    expect(source).not.toContain('remote:${address.href');
  });

  it('invalidates a deferred hidden consent answer when the screen locks', async () => {
    const lock = source.slice(
      source.indexOf("powerMonitor.on('lock-screen'"),
      source.indexOf("globalShortcut.register('CommandOrControl+Alt+Shift+G'")
    );
    expect(lock).toContain(
      "if (anywhereState.phase === 'arming' && anywhereStartSentGeneration !== anywhereGeneration) cancelAnywhere()"
    );
    expect(source).toContain('if (answer.response !== 0 || !stillCurrent()');
    let generation = 7;
    let pendingGeneration: number | null = 7;
    let resolveDialog: (answer: { response: number }) => void = () => {};
    const dialog = new Promise<{ response: number }>((resolve) => {
      resolveDialog = resolve;
    });
    const status = vi.fn();
    const start = vi.fn();
    const waiting = dialog.then((answer) => {
      if (
        answer.response !== 0 ||
        !sameAnywhereStart('tray', 7, generation, 'arming', pendingGeneration, 4, 4, true)
      )
        return;
      status();
      start();
    });
    generation += 1;
    pendingGeneration = null;
    resolveDialog({ response: 0 });
    await waiting;
    expect(status).not.toHaveBeenCalled();
    expect(start).not.toHaveBeenCalled();
    // A preview start does not depend on the clipboard switch, but a screen lock still ends it.
    for (const kind of ['tray', 'shortcut', 'preview'] as const) {
      expect(sameAnywhereStart(kind, 7, generation, 'arming', pendingGeneration, 4, 4, true)).toBe(
        false
      );
    }
  });

  it('keeps the shortcut public and preserves Stop when policy changes', () => {
    const start = source.slice(
      source.indexOf('async function startAnywhere('),
      source.indexOf('function cancelAnywhere()')
    );
    expect(start.indexOf("if (source === 'shortcut') privateConfirmed = false")).toBeLessThan(
      start.indexOf('const generation = ++anywhereGeneration')
    );
    expect(start).toContain('settingsRevision, anywhereSettingsRevision');
    expect(start).toContain('anywhereSettings.enabled && !anywhereOffRequested');
    expect(source).toContain(
      'switchKeepsStart(pending.source, pending.settingsRevision, anywhereSettingsRevision'
    );
    expect(source).toContain('if (!next.enabled) { anywhereOffRequested = true;');
    expect(source).toContain('if (switchGovernsSource(anywhereState.source)) cancelAnywhere();');
    const gate = start.indexOf(
      'if (!canStartAnywhere(anywhereState.phase, source, switchOn) || !laptopDictationCaptureAllowed())'
    );
    expect(gate).toBeGreaterThan(0);
    expect(start.indexOf("if (anywhereState.phase === 'recording')")).toBeLessThan(gate);
  });
});
