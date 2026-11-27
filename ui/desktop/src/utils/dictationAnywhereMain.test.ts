import fs from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vitest';

/**
 * Owner decision, 2026-11-10: **Dictate into this application** works whether or not **Dictate into
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

/**
 * DD-97: the global shortcut and fn in another app type their text into the app that was in front
 * when the dictation started. `dictationAnywhere.test.ts` covers what is typed, copied and shown
 * (`deliverAnywhereTranscript`); these pin how main wires it.
 */
describe('main types a dictation into the app that was in front', () => {
  const start = slice('async function startAnywhere(', 'function cancelAnywhere()');

  it('takes the target at the start, before anything is awaited, and only for the shortcut', () => {
    const generation = start.indexOf('const generation = ++anywhereGeneration');
    const asked = start.indexOf('textInsertService.front()');
    expect(asked).toBeGreaterThan(generation);
    // Nothing between the start and the question waits on anything.
    expect(start.slice(generation, asked)).not.toContain('await ');
    expect(asked).toBeLessThan(start.indexOf('await confirmAnywhereRemote(backend, current)'));
    expect(start).toContain(
      'capturesInsertTarget(source, insertSettings.enabled) && textInsertService ? textInsertService.front() : null'
    );
    expect(start).toContain(
      'anywhereInsertTarget = insertTargetFrom(front, process.pid, generation);'
    );
    // The page learns where the text goes before it gets its start.
    expect(start).toContain(
      'anywherePendingStart.surface = anywhereSurface(source, anywhereInsertTarget !== null);'
    );
    expect(start.indexOf('anywhereInsertTarget = insertTargetFrom(')).toBeLessThan(
      start.indexOf('sendAnywhereStartIfReady();')
    );
    expect(
      slice('function sendAnywhereStartIfReady()', "ipcMain.on('dictation:anywhere:capture-ready'")
    ).toContain('pressedAtEpochMs, surface, });');
  });

  it('types at the finish through one delivery, and never writes the clipboard itself there', () => {
    const reply = slice(
      "ipcMain.on('dictation:anywhere:capture-reply'",
      "ipcMain.handle('dictation:anywhere:get'"
    );
    const finished = reply.slice(
      reply.indexOf("result.type === 'finished'"),
      reply.indexOf("result.type === 'failed'")
    );
    expect(finished).toContain(
      'const insertTarget = anywhereInsertTarget?.generation === result.generation ? anywhereInsertTarget : null; anywhereInsertTarget = null;'
    );
    // Rule changed (F10, DD-102): the delivery's answer now says whether a paste left its text
    // for Copy last dictation, so the one delivery is followed by `.then`.
    expect(finished).toContain(
      'void deliverAnywhereTranscript(typed, insertTarget, anywhereDelivery()).then((delivered) => {'
    );
    expect(finished.split('deliverAnywhereTranscript(').length - 1).toBe(1);
    for (const direct of ['clipboard.writeText', 'clipboardOutcome(', 'showAnywhereReview(']) {
      expect(finished, direct).not.toContain(direct);
    }
    // A failure card takes focus, and a cancel ends the dictation: neither keeps a target.
    expect(reply.slice(reply.indexOf("result.type === 'failed'"))).toContain(
      'anywhereInsertTarget = null;'
    );
    expect(reply.slice(reply.indexOf("result.type === 'cancelled'"))).toContain(
      'anywhereInsertTarget = null;'
    );
    expect(slice('function cancelAnywhere()', 'function isDictationCaptureSender(')).toContain(
      'anywhereInsertTarget = null;'
    );
    const delivery = slice(
      'function anywhereDelivery()',
      'async function insertIntoAnywherePreview('
    );
    expect(delivery).toContain('textInsertService.insert(pid, text)');
    expect(delivery).toContain("Promise.resolve<InsertResult>({ ok: false, reason: 'failed' })");
    expect(delivery).toContain('write: board.write, read: board.read,');
  });

  it('runs the helper with no environment, only while something can type into an app, and stops it at quit', () => {
    const once = slice('function textInsertServiceOnce()', 'function syncDictationHelpers()');
    expect(once).toContain(
      "spawn(file, [...helperArgs], { stdio: ['pipe', 'pipe', 'ignore'], env: {}, windowsHide: true, })"
    );
    expect(once).toContain('helperPath: args === null ? null : textInsertHelperPath(app),');
    expect(once).toContain('const args = textInsertHelperArgs(app);');
    const sync = slice('function syncDictationHelpers()', 'async function insertView()');
    expect(sync).toContain('shortcutRegistered: anywhereShortcut !== null,');
    expect(sync).toContain('if (wanted) service.start(); else service.stop();');
    for (const [handler, end] of [
      [
        "ipcMain.handle('dictation:anywhere:set'",
        '// ── The fn key shortcut and the dictation indicator',
      ],
      ["ipcMain.handle('dictation:fn:set'", "ipcMain.handle('dictation:fn:open-settings'"],
      ["ipcMain.handle('dictation:insert:set'", "ipcMain.handle('dictation:insert:open-settings'"],
    ]) {
      expect(slice(handler, end), handler).toContain('syncDictationHelpers();');
    }
    expect(source).toContain('await readInsertSettings(); syncDictationHelpers();');
    const quit = slice("app.on('will-quit', async () => {", 'clearAnywhereReview();');
    expect(quit).toContain('textInsertService?.stop();');
  });

  it('asks macOS for Accessibility only from the Settings button, and then opens its pane', () => {
    expect(source.split('requestAccess(').length - 1).toBe(1);
    const open = slice(
      "ipcMain.handle('dictation:insert:open-settings'",
      '// ── The theme as words'
    );
    expect(open).toContain(
      "ipcMain.handle('dictation:insert:open-settings', async (event) => { if (!isDictationSender(event)) return;"
    );
    expect(open).toContain('await textInsertServiceOnce()?.requestAccess();');
    expect(open).toContain('Privacy_Accessibility');
    // Settings reads the grant without asking.
    expect(slice('async function insertView()', "ipcMain.handle('dictation:insert:get'")).toContain(
      'await service.access()'
    );
  });

  it('answers the insertion setting only to a Biorouter window, and keeps it in insert.json', () => {
    for (const channel of ['get', 'set', 'open-settings']) {
      const at = source.indexOf(`ipcMain.handle('dictation:insert:${channel}'`);
      expect(at, channel).toBeGreaterThan(0);
      expect(source.slice(at, at + 160), channel).toMatch(
        /=> \{ if \(!isDictationSender\(event\)\) return(?: null)?;/
      );
    }
    const set = slice(
      "ipcMain.handle('dictation:insert:set'",
      "ipcMain.handle('dictation:insert:open-settings'"
    );
    expect(set).toContain("if (typeof enabled !== 'boolean') return await insertView();");
    const write = slice('async function writeInsertSettings(', 'function textInsertServiceOnce()');
    expect(write).toContain('fs.mkdir(path.dirname(filename), { recursive: true, mode: 0o700 })');
    expect(write).toContain('fs.writeFile(temporary, JSON.stringify(settings), { mode: 0o600 })');
    expect(write).toContain('fs.rename(temporary, filename)');
    expect(source).toContain("path.join(biorouterConfigDir(), 'dictation', 'insert.json')");
  });
});

describe('the capture window while fn may dictate into other apps', () => {
  it('stays open between dictations, and closes after its wait once the row is off', () => {
    const idle = slice('function closeCaptureAfterIdle()', 'function holdAnywhereRecording(');
    expect(idle).toContain('anywhereHeld === null && !captureStaysWarm() &&');
    expect(idle).toContain(
      'captureWindowStaysWarm({ platform: process.platform, fnEnabled: fnKeySettings.enabled, fnOtherApps: fnKeySettings.otherApps, anywhereOn: anywhereSettings.enabled && !anywhereOffRequested, })'
    );
    expect(slice('function syncDictationHelpers()', 'async function insertView()')).toContain(
      '!captureStaysWarm() && anywhereCaptureWindow && !anywhereCaptureWindow.isDestroyed() && anywhereAtRest(anywhereState.phase)) closeCaptureAfterIdle();'
    );
  });
});

describe('the dictation test seams reach only an unpackaged app (DESIGN 3.12)', () => {
  it('main reads them once through dictationTestSeams, and spells none of them itself', () => {
    expect(source).toContain('const dictationSeams = dictationTestSeams(app, process.env);');
    for (const variable of [
      'BIOROUTER_FN_TRACE',
      'BIOROUTER_TEST_CLIPBOARD_FILE',
      'BIOROUTER_TEST_INSERT_LOG',
      'BIOROUTER_TEXTINSERT_HELPER',
      'BIOROUTER_TEXTINSERT_TEST_PASTEBOARD',
      'BIOROUTER_FNKEY_HELPER',
    ]) {
      expect(source, variable).not.toContain(variable);
    }
    // The helpers' own overrides are read by their path functions, which take the app.
    expect(source).toContain('helperPath: fnKeyHelperPath(app),');
    expect(source).toContain('textInsertHelperPath(app)');
    expect(source).toContain('textInsertHelperArgs(app)');
  });

  it('the clipboard and the insert log use their files only when a seam names one', () => {
    const board = slice('function anywhereClipboard()', 'function appendInsertLog(');
    // `anywhereClipboardTarget` decides (tested in `dictationAnywhere.test.ts`): a run that names a
    // test pasteboard and no clipboard file writes no clipboard at all.
    expect(source).toContain('const insertTestPasteboard = namesTextInsertTestPasteboard(app);');
    expect(board).toContain(
      'const target = anywhereClipboardTarget(dictationSeams, insertTestPasteboard);'
    );
    expect(board).toContain(
      "if (target.kind === 'system') return { write: (text) => clipboard.writeText(text), read: () => clipboard.readText() };"
    );
    expect(board).toContain(
      "if (target.kind === 'none') return { write: () => undefined, read: () => '' };"
    );
    expect(board).toContain('fsSync.writeFileSync(file, text, { mode: 0o600 })');
    const insertLog = slice('function appendInsertLog(', 'function registerAnywhereShortcut()');
    expect(insertLog).toContain('if (file === null) return;');
    expect(insertLog).toContain('insertLogLine(entry)');
    expect(
      slice(
        "ipcMain.handle('dictation:anywhere:review-copy'",
        "ipcMain.handle('dictation:mic-status'"
      )
    ).toContain(
      'const board = anywhereClipboard(); board.write(text); return board.read() === text;'
    );
  });
});

/**
 * F18 (DD-102): an open History tab hears of every finished dictation, whichever window or app it
 * was made in. Main says so with one word and no payload, to Biorouter windows only, at most every
 * 250 ms; `historyBroadcastDelay` in `dictationAnywhere.ts` is the throttle, tested there.
 */
describe('main tells every window the dictation history changed', () => {
  it('sends no payload, to windowMap windows only, through the throttle', () => {
    const broadcast = slice(
      'function broadcastDictationHistoryChanged()',
      "ipcMain.on('dictation:transcribed'"
    );
    expect(broadcast).toContain(
      'historyBroadcastDelay(historyChangedAt, performance.now(), HISTORY_CHANGED_MIN_MS)'
    );
    expect(broadcast).toContain('for (const win of windowMap.values())');
    expect(broadcast).toContain("win.webContents.send('dictation:history:changed');");
    // The one place it is sent, and never with anything after the channel.
    expect(source.split("'dictation:history:changed'").length - 1).toBe(1);
    expect(source).not.toMatch(/'dictation:history:changed',/);
    expect(source).toContain('const HISTORY_CHANGED_MIN_MS = 250;');
  });

  it('after the capture page’s finished transcript, and after any window’s', () => {
    const reply = slice(
      "ipcMain.on('dictation:anywhere:capture-reply'",
      "ipcMain.handle('dictation:anywhere:get'"
    );
    const finished = reply.slice(
      reply.indexOf("result.type === 'finished'"),
      reply.indexOf("result.type === 'failed'")
    );
    expect(finished).toContain('broadcastDictationHistoryChanged();');
    const transcribed = slice("ipcMain.on('dictation:transcribed'", 'function ');
    expect(transcribed).toContain('if (!isDictationSender(event)) return;');
    expect(transcribed).toContain('broadcastDictationHistoryChanged();');
    expect(slice("app.on('will-quit', async () => {", 'for (const [windowId')).toContain(
      'if (historyChangedTimer) clearTimeout(historyChangedTimer);'
    );
  });
});

/**
 * F13 (DD-102): a start from fn or the global shortcut while the last dictation still transcribes
 * waits, one deep, and begins the moment that transcript arrives; a release or another key for its
 * gesture drops it first. `dictationAnywhere.test.ts` covers the state it moves through.
 */
describe('main starts a queued dictation when the last transcript arrives', () => {
  it('queues only after Stop and Cancel, and only where the state machine says', () => {
    const start = slice('async function startAnywhere(', 'function cancelAnywhere()');
    const queue = start.indexOf(
      'if (queuesAnywhereStart(anywhereState.phase, source, switchOn, anywhereState.queued))'
    );
    expect(queue).toBeGreaterThan(start.indexOf("if (anywhereState.phase === 'recording')"));
    expect(queue).toBeLessThan(
      start.indexOf('if (!canStartAnywhere(anywhereState.phase, source, switchOn)')
    );
  });

  it('reads the queued start before the finish, and begins it after delivering', () => {
    const reply = slice(
      "ipcMain.on('dictation:anywhere:capture-reply'",
      "ipcMain.handle('dictation:anywhere:get'"
    );
    const finished = reply.slice(
      reply.indexOf("result.type === 'finished'"),
      reply.indexOf("result.type === 'failed'")
    );
    const read = finished.indexOf('const queued = anywhereState.queued;');
    const finish = finished.indexOf(
      "anywhereState = nextAnywhereState(anywhereState, { type: 'finish' });"
    );
    const begin = finished.indexOf('if (queued) beginQueuedAnywhere();');
    expect(read).toBeGreaterThan(0);
    expect(finish).toBeGreaterThan(read);
    expect(begin).toBeGreaterThan(finish);
  });

  it('a release or another key for the queued gesture drops it, and leaves the transcription alone', () => {
    const run = slice('function runFnAction(', 'function stopAnywhere(');
    const stop = run.slice(run.indexOf("case 'stop':"), run.indexOf("case 'cancel':"));
    expect(stop).toContain('if (fnOwnsQueue()) {');
    expect(stop).toContain('unqueueAnywhere();');
    const cancel = run.slice(run.indexOf("case 'cancel':"));
    expect(cancel).toContain('if (fnOwnsQueue()) unqueueAnywhere();');
    const unqueue = slice('function unqueueAnywhere()', 'function ');
    expect(unqueue).toContain(
      "anywhereState = nextAnywhereState(anywhereState, { type: 'unqueue' });"
    );
    expect(unqueue).not.toContain('cancelAnywhere');
  });
});

/**
 * F10 and F15 (DD-102): **Copy last dictation** in the tray, for what is left of five minutes after
 * Biorouter typed into another app, or after a new start closed a review card. The text is in
 * main's memory only, and only the person's click writes it to the clipboard.
 */
describe('Copy last dictation', () => {
  it('writes the clipboard only from its click, through the clipboard seam', () => {
    const copy = slice('function copyLastDictation()', 'function ');
    expect(copy).toContain('const text = lastDictationBeforeExpiry(lastDictation, Date.now());');
    expect(copy).toContain('const board = anywhereClipboard();');
    expect(copy).toContain('board.write(text);');
    // The held text is read in two places: that click, and whether the tray offers it.
    expect(source.split('lastDictationBeforeExpiry(lastDictation').length - 1).toBe(2);
    expect(source).toContain(
      'lastDictation: lastDictationBeforeExpiry(lastDictation, Date.now()) !== null,'
    );
    expect(source).toContain('copyLast: copyLastDictation,');
    // No new clipboard writer: the seam's own, the two link copies and the tray's link copy.
    expect(source.split('clipboard.writeText(').length - 1).toBe(4);
  });

  it('holds a paste’s text, and a closed review card’s, and goes when it expires', () => {
    const reply = slice(
      "ipcMain.on('dictation:anywhere:capture-reply'",
      "ipcMain.handle('dictation:anywhere:get'"
    );
    expect(reply).toContain(
      'if (holdsLastDictation(delivered)) holdLastDictation(heldLastDictation(typed.text, Date.now()));'
    );
    const hold = slice('function holdLastDictation(', 'function ');
    expect(hold).toContain('lastDictationTimer = setTimeout(clearLastDictation, wait);');
    const close = slice('function closeAnywhereReviewForStart()', 'function ');
    expect(close).toContain(
      'const handed = reviewHandOff(anywhereReviewText, anywhereReviewExpiresAt, Date.now());'
    );
    expect(close).toContain('if (handed) holdLastDictation(handed);');
    // A new shortcut or fn start closes the card, right after it ends the last one's Retry hold.
    const start = slice('async function startAnywhere(', 'function cancelAnywhere()');
    expect(
      start.indexOf("if (source === 'shortcut') closeAnywhereReviewForStart();")
    ).toBeGreaterThan(start.indexOf('endAnywhereHold();'));
  });

  it('is let go at quit and when Dictate into other apps turns off', () => {
    expect(slice("app.on('will-quit', async () => {", 'for (const [windowId')).toContain(
      'clearLastDictation();'
    );
    const set = slice(
      "ipcMain.handle('dictation:anywhere:set'",
      "ipcMain.handle('dictation:anywhere:preview-focused'"
    );
    const off = set.slice(
      set.indexOf('if (!next.enabled) {'),
      set.indexOf('registerAnywhereShortcut();')
    );
    expect(off).toContain('clearLastDictation();');
  });

  it('rebuilds the tray menu when it starts and stops holding a text, so the item comes and goes', () => {
    // The tray reads `lastDictation` only when its menu is built (`autoUpdater.copyLast.test.ts`).
    const rebuild = 'updateTrayMenu(getUpdateAvailable());';
    const hold = slice('function holdLastDictation(', 'function ');
    expect(hold.indexOf(rebuild)).toBeGreaterThan(hold.indexOf('lastDictation = held;'));
    const clear = slice('function clearLastDictation()', 'function ');
    expect(clear.indexOf(rebuild)).toBeGreaterThan(clear.indexOf('lastDictation = null;'));
    // An expired text found at the click is let go the same way.
    const copy = slice('function copyLastDictation()', 'function ');
    expect(copy).toContain('clearLastDictation();');
  });
});
