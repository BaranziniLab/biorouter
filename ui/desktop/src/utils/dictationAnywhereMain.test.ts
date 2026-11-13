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

/**
 * Owner decision, 2026-11-13: MSG-CLIPBOARD-FAILED offers **Retry** while the failed recording is
 * held, and the plain sentence once the hold ends. `DictationCapturePage.test.tsx` covers the page
 * that keeps the audio, `DictationReviewCard.test.tsx` the card.
 */
describe('main holds a failed clipboard recording for Retry', () => {
  it('offers Retry only for a recording the capture page kept', () => {
    const reply = slice(
      "ipcMain.on('dictation:anywhere:capture-reply'",
      "ipcMain.handle('dictation:anywhere:get'"
    );
    const failed = reply.slice(reply.indexOf("result.type === 'failed'"));
    expect(failed).toContain(
      'const retry = result.held === true && keepsFailedRecording(source, true);'
    );
    expect(failed).toContain(
      'if (retry && source) holdAnywhereRecording(anywhereGeneration, source);'
    );
    expect(failed).toContain(
      "showAnywhereReview('Biorouter could not finish dictation. Try again.', null, false, retry);"
    );
    // Closing the capture window would lose the held audio.
    expect(failed).toContain('if (!retry) closeCaptureAfterIdle();');
  });

  it('never closes the capture window while a recording is held', () => {
    const idle = slice('function closeCaptureAfterIdle()', 'function holdAnywhereRecording(');
    expect(idle).toContain('anywhereHeld === null');
  });

  it('shows Retry only with the failure it was offered for', () => {
    expect(slice('function anywhereReviewPayload()', 'function closeCaptureAfterIdle()')).toContain(
      'retry: anywhereReviewRetry && anywhereHeld !== null'
    );
    const show = slice(
      'async function showAnywhereReview(',
      'async function promptAnywherePrivateTerms('
    );
    expect(show).toContain('anywhereReviewRetry = retry; if (!retry) dropAnywhereHeld();');
  });

  it('ends the hold on time out, a new dictation, Off, a cancel and a closed window', () => {
    expect(slice('function holdAnywhereRecording(', 'function dropAnywhereHeld(')).toContain(
      '}, HELD_FOR_RETRY_MS);'
    );
    expect(slice('function dropAnywhereHeld(', 'function endAnywhereHold(')).toContain(
      "anywhereCaptureWindow.webContents.send('dictation:anywhere:capture-command', { type: 'drop' });"
    );
    const start = slice('async function startAnywhere(', 'function cancelAnywhere()');
    expect(start.indexOf('endAnywhereHold();')).toBeGreaterThan(
      start.indexOf(
        "anywhereState = nextAnywhereState(anywhereState, { type: 'start', source, privateConfirmed });"
      )
    );
    const set = slice(
      "ipcMain.handle('dictation:anywhere:set'",
      "ipcMain.handle('dictation:anywhere:preview-focused'"
    );
    expect(set).toMatch(
      /if \(switchGovernsSource\(anywhereState\.source\)\) cancelAnywhere\(\); endAnywhereHold\(\);/
    );
    expect(slice('function cancelAnywhere()', 'function isDictationCaptureSender(')).toContain(
      'endAnywhereHold();'
    );
    const closed = slice(
      'async function createAnywhereWindow(',
      'async function showAnywhereReview('
    );
    const captureClosed = closed.slice(closed.indexOf('if (anywhereCaptureWindow === win) {'));
    expect(
      captureClosed.slice(0, captureClosed.indexOf('if (anywhereReviewWindow === win)'))
    ).toContain('endAnywhereHold();');
    expect(closed.slice(closed.indexOf('if (anywhereReviewWindow === win) {'))).toContain(
      'anywhereReviewRetry = false; dropAnywhereHeld();'
    );
  });

  it('runs a Retry only for the current hold, at rest, under the policy and backend it began with', () => {
    const retry = slice(
      'function retryAnywhere()',
      "ipcMain.handle('dictation:anywhere:review-retry'"
    );
    for (const condition of [
      'held.generation !== anywhereGeneration',
      '!canStartAnywhere(anywhereState.phase, held.source, anywhereSettings.enabled && !anywhereOffRequested)',
      '!laptopDictationCaptureAllowed()',
      '!windowOnAnywhereBackend(captureWindow, backend)',
    ]) {
      expect(retry).toContain(condition);
    }
    // Refused, the hold ends and the card keeps its sentence without Retry.
    expect(retry.indexOf('endAnywhereHold(); return false;')).toBeGreaterThan(0);
    expect(retry).toContain(
      "nextAnywhereState(anywhereState, { type: 'retry', source: held.source })"
    );
    expect(retry).toContain(
      "captureWindow.webContents.send('dictation:anywhere:capture-command', { type: 'retry' });"
    );
    expect(retry.indexOf('endAnywhereHold(); return false;')).toBeLessThan(
      retry.indexOf("{ type: 'retry' });")
    );
  });
});
