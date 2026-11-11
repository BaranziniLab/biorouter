import fs from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vitest';

/**
 * Owner decision, 2026-11-10: **Dictate into this app** works whether or not **Dictate into
 * other apps** is on; the switch governs the tray item and the global shortcut only. Electron
 * main cannot be loaded in jsdom, so these read its source; `dictationAnywhere.test.ts` covers
 * the functions it calls.
 */
// Each run of white space as one space and none just inside a parenthesis, so the literals
// below pin what the code says and not where Prettier breaks its lines.
const source = fs
  .readFileSync(path.join(__dirname, '../main.ts'), 'utf8')
  .replace(/\s+/g, ' ')
  .replace(/\( /g, '(')
  .replace(/ \)/g, ')');

function slice(from: string, to: string): string {
  const start = source.indexOf(from);
  const end = source.indexOf(to, start + 1);
  expect(start, from).toBeGreaterThan(0);
  expect(end, to).toBeGreaterThan(start);
  return source.slice(start, end);
}

describe('main applies the switch to the tray item and the shortcut only', () => {
  it('lets a preview start pass the switch and refuses only a new shortcut press up front', () => {
    const start = slice('async function startAnywhere(', 'function cancelAnywhere()');
    expect(start).not.toContain('!anywhereSettings.enabled || anywhereOffRequested');
    expect(start).toContain("source === 'shortcut' && !switchOn");
    const gate = start.indexOf('if (!canStartAnywhere(anywhereState.phase, source, switchOn)');
    expect(gate).toBeGreaterThan(0);
    // Stop and Cancel come first, so the tray's Stop and copy still ends a preview dictation
    // while the switch is off.
    expect(start.indexOf("if (anywhereState.phase === 'arming')")).toBeLessThan(gate);
    expect(start.indexOf("if (anywhereState.phase === 'recording')")).toBeLessThan(gate);
    expect(gate).toBeLessThan(start.indexOf('const generation = ++anywhereGeneration'));
    expect(start).toContain('sameAnywhereStart(source, generation,');
  });

  it('sends a pending start to the capture window by the same rule', () => {
    const send = slice(
      'function sendAnywhereStartIfReady()',
      "ipcMain.on('dictation:anywhere:capture-ready'"
    );
    expect(send).toContain(
      'switchKeepsStart(pending.source, pending.settingsRevision, anywhereSettingsRevision'
    );
    expect(send).not.toContain('anywhereOffRequested || !anywhereSettings.enabled');
  });

  it('cancels only a tray or shortcut dictation when the switch is turned off', () => {
    const set = slice(
      "ipcMain.handle('dictation:anywhere:set'",
      "ipcMain.handle('dictation:anywhere:preview-focused'"
    );
    expect(set).toContain('if (switchGovernsSource(anywhereState.source)) cancelAnywhere();');
    expect(set).not.toMatch(/anywhereOffRequested = true; cancelAnywhere\(\);/);
  });

  it('asks nothing about the switch before a preview start', () => {
    const preview = slice(
      "ipcMain.handle('dictation:anywhere:preview-start'",
      "ipcMain.handle('dictation:anywhere:review-get'"
    );
    expect(preview).not.toContain('anywhereSettings.enabled');
    expect(preview).not.toContain('anywhereOffRequested');
    expect(preview).toContain("startAnywhere('preview', privateConfirmed, { owner, viewId })");
  });

  it('closes an idle capture window whether the switch is on or off', () => {
    const idle = slice('function closeCaptureAfterIdle()', 'function stopAnywherePowerBlocker()');
    expect(idle).toContain('anywhereAtRest(anywhereState.phase)');
    expect(idle).not.toContain("anywhereState.phase === 'idle'");
  });
});
