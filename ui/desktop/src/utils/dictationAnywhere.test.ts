import { describe, expect, it, vi } from 'vitest';
import type { Transcript } from '../components/dictation/dictationApi';
import {
  dictationMessage,
  insertRefusalMessage,
  insertRestoreMessage,
} from '../components/dictation/copy';
import {
  INSERT_RESTORED,
  MAX_REQUEST_LINE_BYTES,
  TextInsertService,
  type InsertRefusal,
  type InsertResult,
} from './textInsertHelper';
import {
  historyBroadcastDelay,
  heldLastDictation,
  holdsLastDictation,
  lastDictationBeforeExpiry,
  reviewHandOff,
  anywhereAtRest,
  anywhereSurface,
  capturesInsertTarget,
  captureWindowStaysWarm,
  defaultInsertSettings,
  deliverAnywhereTranscript,
  DICTATION_TEST_SEAM_ENV,
  dictationTestSeams,
  insertableText,
  insertLogLine,
  insertTargetFrom,
  parseAnywhereSurface,
  parseInsertSettings,
  textInsertWanted,
  type AnywhereDelivery,
  type InsertLogEntry,
  type InsertTarget,
  canStartAnywhere,
  clipboardOutcome,
  DEFAULT_ANYWHERE_SHORTCUT,
  externalStreamRequest,
  INITIAL_ANYWHERE_STATE,
  nextAnywhereState,
  parseAnywhereSettings,
  REVIEW_TEXT_MS,
  reviewTextBeforeExpiry,
  sameAnywhereStart,
  sameCaptureOperation,
  switchGovernsSource,
  switchKeepsStart,
  currentAnywhereSettingsRequest,
  usableAnywhereTrigger,
  elapsedFromPress,
  abortFinishingStream,
  deliverSettingsWhenReady,
  afterAnywhereSettingsWriteFailure,
  keepsFailedRecording,
  type AnywhereSettings,
} from './dictationAnywhere';

const transcript = (parts: Partial<Transcript> = {}): Transcript => ({
  text: 'hello world',
  raw_text: 'hello world',
  changes: [],
  segments: [],
  model: 'test',
  audio_s: 1,
  elapsed_ms: 1,
  bias_scope: 'public',
  retried: null,
  no_speech: false,
  dictionary_only: false,
  ...parts,
});

describe('dictation anywhere', () => {
  it('never gives the global shortcut private terms', () => {
    expect(externalStreamRequest(16000, 'shortcut', true).include_private_terms).toBeUndefined();
    expect(externalStreamRequest(16000, 'tray', false).include_private_terms).toBeUndefined();
    expect(externalStreamRequest(16000, 'tray', true).include_private_terms).toBe(true);
    expect(externalStreamRequest(16000, 'preview', true).target_kind).toBe('external');
    const armed = nextAnywhereState(nextAnywhereState(INITIAL_ANYWHERE_STATE, { type: 'enable' }), {
      type: 'start',
      source: 'shortcut',
      privateConfirmed: true,
    });
    expect(armed.includePrivateTerms).toBe(false);
    expect(nextAnywhereState(armed, { type: 'stop' }).includePrivateTerms).toBe(false);
  });

  it('leaves the clipboard unchanged for authoritative no_speech or empty final text', () => {
    const write = vi.fn();
    const read = vi.fn();
    expect(clipboardOutcome(transcript({ no_speech: true }), write, read)).toEqual({
      kind: 'nothing',
    });
    expect(clipboardOutcome(transcript({ text: '   ' }), write, read)).toEqual({ kind: 'nothing' });
    expect(write).not.toHaveBeenCalled();
    expect(read).not.toHaveBeenCalled();
  });

  it('does not write dictionary-only text or accept a missing marker', () => {
    const write = vi.fn();
    const read = vi.fn();
    expect(
      clipboardOutcome(
        transcript({ text: 'EPAS1', changes: [], dictionary_only: true }),
        write,
        read
      )
    ).toEqual({ kind: 'echo' });
    expect(clipboardOutcome(transcript({ dictionary_only: undefined }), write, read)).toEqual({
      kind: 'old-backend',
    });
    expect(write).not.toHaveBeenCalled();
    expect(read).not.toHaveBeenCalled();
  });

  it('checks clipboard readback and retains a failed copy only for five minutes', () => {
    const write = vi.fn();
    expect(clipboardOutcome(transcript(), write, () => 'other')).toEqual({
      kind: 'review',
      text: 'hello world',
    });
    expect(write).toHaveBeenCalledWith('hello world');
    const enabled = nextAnywhereState(INITIAL_ANYWHERE_STATE, { type: 'enable' });
    const armed = nextAnywhereState(enabled, {
      type: 'start',
      source: 'tray',
      privateConfirmed: true,
    });
    const stopped = nextAnywhereState(armed, { type: 'stop' });
    const review = nextAnywhereState(stopped, { type: 'review', text: 'hello world', now: 100 });
    expect(review.reviewText).toBe('hello world');
    expect(
      nextAnywhereState(review, { type: 'expire', now: 100 + REVIEW_TEXT_MS }).reviewText
    ).toBeNull();
    expect(nextAnywhereState(review, { type: 'cancel' }).reviewText).toBeNull();
  });

  it('copies only after a speech result and reports a count without text', () => {
    const write = vi.fn();
    expect(clipboardOutcome(transcript(), write, () => 'hello world')).toEqual({
      kind: 'copied',
      words: 2,
    });
    expect(write).toHaveBeenCalledWith('hello world');
    expect(
      clipboardOutcome(
        transcript({ changes: [{ start: 0, end: 5, from: '', to: 'hello', stage: 'exact' }] }),
        write,
        () => 'hello world'
      )
    ).toEqual({ kind: 'copied', words: 2 });
    expect(
      clipboardOutcome(
        transcript({
          retried: { reason: 'echo' },
          changes: [{ start: 0, end: 11, from: '', to: 'hello world', stage: 'exact' }],
        }),
        write,
        () => 'hello world'
      )
    ).toEqual({ kind: 'copied', words: 2 });
  });

  it('enforces absolute review expiry at the boundary', () => {
    const expiry = 1_000 + REVIEW_TEXT_MS;
    expect(reviewTextBeforeExpiry('EPAS1', expiry, expiry - 1)).toBe('EPAS1');
    expect(reviewTextBeforeExpiry('EPAS1', expiry, expiry)).toBeNull();
    expect(reviewTextBeforeExpiry('EPAS1', expiry, expiry + 1)).toBeNull();
    expect(reviewTextBeforeExpiry('EPAS1', null, 1_000)).toBeNull();
  });

  it('does not resume a deferred start after cancel or replacement', async () => {
    let release!: () => void;
    const loading = new Promise<void>((resolve) => {
      release = resolve;
    });
    let generation = 3;
    const pendingGeneration = 3;
    const waiting = loading.then(() =>
      sameAnywhereStart('tray', 3, generation, 'arming', pendingGeneration, 1, 1, true)
    );
    generation += 1;
    release();
    expect(await waiting).toBe(false);
    for (const source of ['tray', 'shortcut', 'preview'] as const) {
      expect(sameAnywhereStart(source, 3, 4, 'arming', 3, 1, 1, true)).toBe(false);
      expect(sameAnywhereStart(source, 3, 3, 'idle', 3, 1, 1, true)).toBe(false);
      expect(sameAnywhereStart(source, 3, 3, 'arming', null, 1, 1, true)).toBe(false);
      expect(sameAnywhereStart(source, 3, 3, 'arming', 3, 1, 1, true)).toBe(true);
    }
  });

  it('refuses a microphone start after deferred status resolves past cancel', async () => {
    let release!: () => void;
    const status = new Promise<void>((resolve) => {
      release = resolve;
    });
    let operation = 7;
    let cancelled = false;
    const wouldOpenMicrophone = status.then(() => sameCaptureOperation(7, operation, cancelled));
    operation += 1;
    cancelled = true;
    release();
    expect(await wouldOpenMicrophone).toBe(false);
    expect(sameCaptureOperation(7, 7, false)).toBe(true);
  });

  it('rejects a delayed shortcut enable after an explicit off request', async () => {
    let release!: () => void;
    const availability = new Promise<void>((resolve) => {
      release = resolve;
    });
    let revision = 0;
    const enable = ++revision;
    const late = availability.then(() => currentAnywhereSettingsRequest(enable, revision));
    const disable = ++revision;
    expect(currentAnywhereSettingsRequest(disable, revision)).toBe(true);
    release();
    expect(await late).toBe(false);
  });

  it('requires a usable tray or registered shortcut and measures the full press path', () => {
    expect(usableAnywhereTrigger(false, DEFAULT_ANYWHERE_SHORTCUT, true, false)).toBe(false);
    expect(usableAnywhereTrigger(false, DEFAULT_ANYWHERE_SHORTCUT, false, false)).toBe(false);
    expect(usableAnywhereTrigger(false, DEFAULT_ANYWHERE_SHORTCUT, false, true)).toBe(true);
    expect(usableAnywhereTrigger(true, null, true, false)).toBe(true);
    expect(elapsedFromPress(1_000, 1_230)).toBe(230);
  });

  it('waits for the exact new renderer and preserves a newer navigation intent', async () => {
    let ready!: () => void;
    const rendererReady = new Promise<void>((resolve) => {
      ready = resolve;
    });
    let latest = 4;
    const waiting = rendererReady.then(() => deliverSettingsWhenReady(4, latest, true, true));
    expect(deliverSettingsWhenReady(4, latest, false, true)).toBe(false);
    expect(deliverSettingsWhenReady(4, latest, true, false)).toBe(false);
    latest = 5;
    ready();
    expect(await waiting).toBe(false);
    expect(deliverSettingsWhenReady(5, latest, true, true)).toBe(true);
  });

  it('aborts a detached finish and ignores its late result after cancel', async () => {
    let settle!: (value: string) => void;
    const finish = new Promise<string>((resolve) => {
      settle = resolve;
    });
    const cancel = vi.fn(async () => undefined);
    let generation = 5;
    const pending = finish.then((text) =>
      sameCaptureOperation(5, generation, false) ? text : null
    );
    abortFinishingStream({ cancel });
    generation += 1;
    expect(cancel).toHaveBeenCalledOnce();
    settle('late transcript');
    expect(await pending).toBeNull();
  });

  it('does not start the microphone when off arrives during availability', async () => {
    let release!: () => void;
    const availability = new Promise<void>((resolve) => {
      release = resolve;
    });
    const startMicrophone = vi.fn();
    let generation = 4;
    let settingsRevision = 8;
    let enabled = true;
    const pending = availability.then(() => {
      if (sameAnywhereStart('tray', 4, generation, 'arming', 4, 8, settingsRevision, enabled))
        startMicrophone();
    });
    settingsRevision += 1;
    enabled = false;
    generation += 1;
    release();
    await pending;
    expect(startMicrophone).not.toHaveBeenCalled();
    for (const source of ['tray', 'shortcut'] as const) {
      expect(sameAnywhereStart(source, 4, 4, 'arming', 4, 8, 9, true)).toBe(false);
      expect(sameAnywhereStart(source, 4, 4, 'arming', 4, 8, 8, false)).toBe(false);
    }
  });

  it('keeps runtime off after an older enable write and the off write both fail', async () => {
    const disabled = { enabled: false, shortcut: 'Control+Alt+D', disclosureShown: true };
    const enabled = { ...disabled, enabled: true };
    let runtime: { settings: AnywhereSettings; offRequested: boolean } = {
      settings: enabled,
      offRequested: false,
    };
    let latestRevision = 1;
    let rejectOlder!: (error: Error) => void;
    const olderWrite = new Promise<void>((_resolve, reject) => {
      rejectOlder = reject;
    }).catch(() => {
      runtime = afterAnywhereSettingsWriteFailure(
        disabled,
        false,
        runtime.settings,
        runtime.offRequested,
        1,
        latestRevision,
        true
      );
    });
    latestRevision = 2;
    runtime.offRequested = true;
    const shortcutRegistered = vi.fn(() => runtime.settings.enabled && !runtime.offRequested);
    expect(shortcutRegistered()).toBe(false);
    rejectOlder(new Error('older write failed'));
    await olderWrite;
    expect(runtime.settings.enabled).toBe(false);
    const failedOff = Promise.reject(new Error('off write failed')).catch(() => {
      runtime = afterAnywhereSettingsWriteFailure(
        enabled,
        true,
        runtime.settings,
        runtime.offRequested,
        2,
        latestRevision,
        false
      );
    });
    await failedOff;
    expect(runtime).toMatchObject({ settings: { enabled: false }, offRequested: true });
    expect(shortcutRegistered()).toBe(false);
  });

  it('fails closed on bad settings and clears private choice on refusal', () => {
    expect(parseAnywhereSettings({ enabled: 'true', shortcut: 4 })).toMatchObject({
      enabled: false,
      shortcut: null,
    });
    const enabled = nextAnywhereState(INITIAL_ANYWHERE_STATE, { type: 'enable' });
    const armed = nextAnywhereState(enabled, {
      type: 'start',
      source: 'tray',
      privateConfirmed: true,
    });
    for (const reason of ['shortcut-taken', 'policy', 'no-model'] as const) {
      expect(nextAnywhereState(armed, { type: 'refuse', reason })).toMatchObject({
        phase: 'idle',
        includePrivateTerms: false,
        reason,
      });
    }
  });
});

// Owner decision, 2026-11-10: Dictate into this application works whether or not Dictate into other
// apps is on. The switch governs the tray item and the global shortcut only.
describe('the Dictate into other apps switch', () => {
  it('governs the tray item and the global shortcut, not the preview button', () => {
    expect(switchGovernsSource('tray')).toBe(true);
    expect(switchGovernsSource('shortcut')).toBe(true);
    expect(switchGovernsSource('preview')).toBe(false);
    expect(switchGovernsSource(null)).toBe(false);
  });

  it('lets a preview dictation start and finish with the switch off', () => {
    expect(canStartAnywhere('off', 'preview', false)).toBe(true);
    const armed = nextAnywhereState(INITIAL_ANYWHERE_STATE, {
      type: 'start',
      source: 'preview',
      privateConfirmed: false,
    });
    expect(armed).toMatchObject({ phase: 'arming', source: 'preview', switchOn: false });
    const recording = nextAnywhereState(armed, { type: 'capture-ready' });
    expect(recording.phase).toBe('recording');
    const transcribing = nextAnywhereState(recording, { type: 'stop' });
    expect(transcribing.phase).toBe('transcribing');
    const done = nextAnywhereState(transcribing, { type: 'finish' });
    expect(done).toMatchObject({ phase: 'off', switchOn: false, source: null });
    expect(anywhereAtRest(done.phase)).toBe(true);
    for (const event of [
      { type: 'cancel' },
      { type: 'refuse', reason: 'capture-failed' },
    ] as const) {
      expect(nextAnywhereState(armed, event)).toMatchObject({ phase: 'off', switchOn: false });
    }
  });

  it('still refuses the tray item and the shortcut with the switch off', () => {
    for (const source of ['tray', 'shortcut'] as const) {
      expect(canStartAnywhere('off', source, false)).toBe(false);
      // Main's switch is already off while the off write is queued and the state is still idle.
      expect(canStartAnywhere('idle', source, false)).toBe(false);
      expect(
        nextAnywhereState(INITIAL_ANYWHERE_STATE, {
          type: 'start',
          source,
          privateConfirmed: false,
        })
      ).toBe(INITIAL_ANYWHERE_STATE);
      expect(canStartAnywhere('idle', source, true)).toBe(true);
    }
    const afterPreview = nextAnywhereState(
      nextAnywhereState(INITIAL_ANYWHERE_STATE, {
        type: 'start',
        source: 'preview',
        privateConfirmed: false,
      }),
      { type: 'finish' }
    );
    expect(
      nextAnywhereState(afterPreview, { type: 'start', source: 'tray', privateConfirmed: false })
        .phase
    ).toBe('off');
  });

  it('keeps a pending preview start through an off request and drops a tray or shortcut one', async () => {
    let release!: () => void;
    const availability = new Promise<void>((resolve) => {
      release = resolve;
    });
    let settingsRevision = 8;
    let switchOn = true;
    const startMicrophone = vi.fn();
    const pending = availability.then(() => {
      for (const source of ['tray', 'shortcut', 'preview'] as const) {
        if (sameAnywhereStart(source, 4, 4, 'arming', 4, 8, settingsRevision, switchOn))
          startMicrophone(source);
      }
    });
    settingsRevision += 1;
    switchOn = false;
    release();
    await pending;
    expect(startMicrophone.mock.calls).toEqual([['preview']]);
    expect(switchKeepsStart('preview', 1, 2, false)).toBe(true);
    expect(switchKeepsStart('tray', 1, 1, false)).toBe(false);
    expect(switchKeepsStart('shortcut', 1, 2, true)).toBe(false);
    expect(switchKeepsStart('tray', 2, 2, true)).toBe(true);
  });

  it('lets a preview dictation run on when the switch is turned off, and ends a tray one', () => {
    const on = nextAnywhereState(INITIAL_ANYWHERE_STATE, { type: 'enable' });
    const preview = nextAnywhereState(
      nextAnywhereState(on, { type: 'start', source: 'preview', privateConfirmed: true }),
      { type: 'capture-ready' }
    );
    const previewAfterOff = nextAnywhereState(preview, { type: 'disable' });
    expect(previewAfterOff).toMatchObject({
      phase: 'recording',
      source: 'preview',
      switchOn: false,
      includePrivateTerms: true,
    });
    expect(
      nextAnywhereState(nextAnywhereState(previewAfterOff, { type: 'stop' }), { type: 'finish' })
        .phase
    ).toBe('off');

    const tray = nextAnywhereState(
      nextAnywhereState(on, { type: 'start', source: 'tray', privateConfirmed: false }),
      { type: 'capture-ready' }
    );
    expect(nextAnywhereState(tray, { type: 'disable' })).toEqual(INITIAL_ANYWHERE_STATE);
  });

  it('brings a preview dictation to rest as idle when the switch is turned on meanwhile', () => {
    const armed = nextAnywhereState(INITIAL_ANYWHERE_STATE, {
      type: 'start',
      source: 'preview',
      privateConfirmed: false,
    });
    const turnedOn = nextAnywhereState(armed, { type: 'enable' });
    expect(turnedOn).toMatchObject({ phase: 'arming', source: 'preview', switchOn: true });
    expect(nextAnywhereState(turnedOn, { type: 'finish' }).phase).toBe('idle');
  });

  it('starts no second dictation while one is in progress, from any source', () => {
    for (const phase of ['arming', 'recording', 'transcribing', 'review'] as const) {
      for (const source of ['tray', 'shortcut', 'preview'] as const) {
        expect(canStartAnywhere(phase, source, true)).toBe(false);
      }
    }
  });
});

// Owner decision, 2026-11-13: a failed clipboard dictation offers Retry while its recording is held.
describe('a recording held for Retry', () => {
  it('is kept only for a tray or shortcut dictation the stream can send again', () => {
    expect(keepsFailedRecording('tray', true)).toBe(true);
    expect(keepsFailedRecording('shortcut', true)).toBe(true);
    // The field a preview dictation was for may hold other text five minutes later.
    expect(keepsFailedRecording('preview', true)).toBe(false);
    expect(keepsFailedRecording(null, true)).toBe(false);
    for (const source of ['tray', 'shortcut', 'preview'] as const) {
      expect(keepsFailedRecording(source, false)).toBe(false);
    }
  });

  it('is transcribed again from rest, as a new start from its source could begin', () => {
    const on = nextAnywhereState(INITIAL_ANYWHERE_STATE, { type: 'enable' });
    const failed = nextAnywhereState(
      nextAnywhereState(
        nextAnywhereState(on, { type: 'start', source: 'tray', privateConfirmed: true }),
        { type: 'stop' }
      ),
      { type: 'refuse', reason: 'capture-failed' }
    );
    expect(failed).toMatchObject({ phase: 'idle', source: null });
    const retrying = nextAnywhereState(failed, { type: 'retry', source: 'tray' });
    expect(retrying).toMatchObject({
      phase: 'transcribing',
      source: 'tray',
      switchOn: true,
      includePrivateTerms: false,
    });
    // The switch's Off still ends it, and it comes to rest as a first finish does.
    expect(nextAnywhereState(retrying, { type: 'disable' })).toEqual(INITIAL_ANYWHERE_STATE);
    expect(nextAnywhereState(retrying, { type: 'finish' }).phase).toBe('idle');
    // Nothing is retried while another dictation runs, or with the switch off.
    const recording = nextAnywhereState(
      nextAnywhereState(on, { type: 'start', source: 'shortcut', privateConfirmed: false }),
      { type: 'capture-ready' }
    );
    expect(nextAnywhereState(recording, { type: 'retry', source: 'tray' })).toBe(recording);
    expect(nextAnywhereState(INITIAL_ANYWHERE_STATE, { type: 'retry', source: 'shortcut' })).toBe(
      INITIAL_ANYWHERE_STATE
    );
  });
});

describe('where the text goes, for the dictation history', () => {
  it('records the app in front only for the shortcut, the preview always as a preview', () => {
    expect(externalStreamRequest(16000, 'shortcut', false, 'other_app').surface).toBe('other_app');
    expect(externalStreamRequest(16000, 'shortcut', false, 'clipboard').surface).toBe('clipboard');
    expect(externalStreamRequest(16000, 'shortcut', false).surface).toBe('clipboard');
    // The tray never types into another app, whatever it is told.
    expect(externalStreamRequest(16000, 'tray', false, 'other_app').surface).toBe('clipboard');
    expect(externalStreamRequest(16000, 'preview', false, 'other_app').surface).toBe('app_preview');
    expect(externalStreamRequest(16000, 'preview', false).surface).toBe('app_preview');
    expect(anywhereSurface('shortcut', true)).toBe('other_app');
    expect(anywhereSurface('tray', false)).toBe('clipboard');
    expect(anywhereSurface('preview', true)).toBe('app_preview');
  });

  it('reads a surface word from main and nothing else', () => {
    for (const word of ['other_app', 'clipboard', 'app_preview'] as const) {
      expect(parseAnywhereSurface(word)).toBe(word);
    }
    for (const other of ['composer', 'terminal', 'OTHER_APP', '', null, undefined, 3, {}]) {
      expect(parseAnywhereSurface(other)).toBeNull();
    }
  });
});

describe('typing the text into the app in front (DD-97)', () => {
  const target: InsertTarget = { generation: 4, pid: 812, bundle: 'com.apple.TextEdit' };

  function delivery(insert: (pid: number, text: string) => Promise<InsertResult>, read?: string) {
    let board = '';
    const deps = {
      insert: vi.fn(insert),
      write: vi.fn((text: string) => {
        board = text;
      }),
      read: vi.fn(() => read ?? board),
      notify: vi.fn(),
      review: vi.fn(),
      pasteKey: '⌘V',
      oldBackendMessage: 'old daemon',
      log: vi.fn((entry: InsertLogEntry) => entry),
    } satisfies AnywhereDelivery;
    return deps;
  }

  it('is on by default on macOS only, and written only as a boolean', () => {
    expect(defaultInsertSettings('darwin')).toEqual({ enabled: true });
    expect(defaultInsertSettings('linux')).toEqual({ enabled: false });
    expect(parseInsertSettings(null, 'darwin')).toEqual({ enabled: true });
    expect(parseInsertSettings({ enabled: false }, 'darwin')).toEqual({ enabled: false });
    expect(parseInsertSettings({ enabled: 'yes' }, 'darwin')).toEqual({ enabled: true });
    expect(parseInsertSettings({ enabled: true }, 'win32')).toEqual({ enabled: false });
    expect(parseInsertSettings([true], 'darwin')).toEqual({ enabled: true });
  });

  it('takes a target only for the global shortcut and fn in another app, never the tray or a preview', () => {
    expect(capturesInsertTarget('shortcut', true)).toBe(true);
    expect(capturesInsertTarget('tray', true)).toBe(false);
    expect(capturesInsertTarget('preview', true)).toBe(false);
    expect(capturesInsertTarget('shortcut', false)).toBe(false);
  });

  it('keeps the app in front as the target, unless it is Biorouter itself or unknown', () => {
    expect(insertTargetFrom({ pid: 812, bundle: 'com.apple.TextEdit' }, 100, 4)).toEqual(target);
    expect(insertTargetFrom({ pid: 100, bundle: 'com.ucsf.biorouter' }, 100, 4)).toBeNull();
    expect(insertTargetFrom(null, 100, 4)).toBeNull();
    expect(insertTargetFrom({ pid: 0, bundle: null }, 100, 4)).toBeNull();
    expect(insertTargetFrom({ pid: 1.5, bundle: null }, 100, 4)).toBeNull();
    expect(insertTargetFrom({ pid: 9, bundle: null }, 100, 4)).toEqual({
      generation: 4,
      pid: 9,
      bundle: null,
    });
  });

  it('runs the helper only while something can start a dictation that types into an app', () => {
    const on = {
      platform: 'darwin',
      insertEnabled: true,
      anywhereOn: true,
      fnOtherApps: true,
      shortcutRegistered: false,
    };
    expect(textInsertWanted(on)).toBe(true);
    expect(textInsertWanted({ ...on, fnOtherApps: false, shortcutRegistered: true })).toBe(true);
    expect(textInsertWanted({ ...on, fnOtherApps: false })).toBe(false);
    expect(textInsertWanted({ ...on, insertEnabled: false })).toBe(false);
    expect(textInsertWanted({ ...on, anywhereOn: false })).toBe(false);
    expect(textInsertWanted({ ...on, platform: 'linux' })).toBe(false);
  });

  it('types real speech only, and leaves an echo, nothing heard and an older daemon alone', () => {
    expect(insertableText(transcript())).toBe('hello world');
    expect(insertableText(transcript({ no_speech: true }))).toBeNull();
    expect(insertableText(transcript({ text: '  ' }))).toBeNull();
    expect(insertableText(transcript({ dictionary_only: true }))).toBeNull();
    expect(insertableText(transcript({ dictionary_only: undefined }))).toBeNull();
  });

  it('with a target, types the text and never writes the clipboard or shows anything', async () => {
    const deps = delivery(async () => ({ ok: true }));
    expect(await deliverAnywhereTranscript(transcript(), target, deps)).toEqual({
      kind: 'inserted',
    });
    expect(deps.insert).toHaveBeenCalledWith(812, 'hello world');
    expect(deps.write).not.toHaveBeenCalled();
    expect(deps.read).not.toHaveBeenCalled();
    expect(deps.notify).not.toHaveBeenCalled();
    expect(deps.review).not.toHaveBeenCalled();
  });

  it.each<InsertRefusal>(['no-permission', 'secure-input', 'target-changed', 'failed'])(
    'copies when the helper refuses with %s, and the notification says why',
    async (reason) => {
      const deps = delivery(async () => ({ ok: false, reason }));
      expect(await deliverAnywhereTranscript(transcript(), target, deps)).toEqual({
        kind: 'copied',
        reason,
      });
      expect(deps.write).toHaveBeenCalledExactlyOnceWith('hello world');
      expect(deps.notify).toHaveBeenCalledExactlyOnceWith(insertRefusalMessage(reason, '⌘V'));
      // The card never takes focus from the app while the copy is good.
      expect(deps.review).not.toHaveBeenCalled();
    }
  );

  it('treats a helper that throws as a failed insert, and copies', async () => {
    const deps = delivery(async () => {
      throw new Error('gone');
    });
    expect(await deliverAnywhereTranscript(transcript(), target, deps)).toEqual({
      kind: 'copied',
      reason: 'failed',
    });
    expect(deps.notify).toHaveBeenCalledWith(insertRefusalMessage('failed', '⌘V'));
  });

  it('opens the review card only when the fallback copy did not stick', async () => {
    const deps = delivery(async () => ({ ok: false, reason: 'secure-input' }), 'something else');
    expect(await deliverAnywhereTranscript(transcript(), target, deps)).toEqual({
      kind: 'review',
      reason: 'secure-input',
    });
    expect(deps.review).toHaveBeenCalledExactlyOnceWith(
      'The clipboard did not keep the text. Copy it from this card.',
      'hello world'
    );
    expect(deps.notify).not.toHaveBeenCalled();
  });

  it('without a target, copies as before', async () => {
    const deps = delivery(async () => ({ ok: true }));
    expect(await deliverAnywhereTranscript(transcript(), null, deps)).toEqual({ kind: 'copied' });
    expect(deps.insert).not.toHaveBeenCalled();
    expect(deps.notify).toHaveBeenCalledExactlyOnceWith(
      dictationMessage('MSG-CLIPBOARD-COPIED', { n: '2', key: '⌘V' })
    );
  });

  it('keeps the echo, nothing-heard and older-daemon messages, and touches neither the clipboard nor the app', async () => {
    for (const [parts, kind] of [
      [{ dictionary_only: true }, 'echo'],
      [{ no_speech: true }, 'nothing'],
      [{ dictionary_only: undefined }, 'old-backend'],
    ] as const) {
      const deps = delivery(async () => ({ ok: true }));
      expect(await deliverAnywhereTranscript(transcript(parts), target, deps)).toEqual({ kind });
      expect(deps.insert).not.toHaveBeenCalled();
      expect(deps.write).not.toHaveBeenCalled();
      if (kind === 'old-backend') expect(deps.review).toHaveBeenCalledWith('old daemon', null);
      else expect(deps.review).not.toHaveBeenCalled();
    }
    const echo = delivery(async () => ({ ok: true }));
    await deliverAnywhereTranscript(transcript({ dictionary_only: true }), target, echo);
    expect(echo.notify).toHaveBeenCalledWith(dictationMessage('MSG-CLIPBOARD-ECHO'));
  });

  it('logs each attempt with its route, method, app and length, and never the text', async () => {
    const deps = delivery(async () => ({ ok: false, reason: 'target-changed' }));
    await deliverAnywhereTranscript(
      transcript({ text: 'EPAS1 variant', raw_text: 'EPAS1 variant' }),
      target,
      deps
    );
    const entries = deps.log.mock.calls.map(([entry]) => entry);
    expect(entries).toEqual([
      {
        route: 'other_app',
        method: 'paste',
        bundle: 'com.apple.TextEdit',
        result: 'target-changed',
        chars: 13,
      },
      {
        route: 'other_app',
        method: 'copy',
        bundle: 'com.apple.TextEdit',
        result: 'copied',
        reason: 'target-changed',
        chars: 13,
      },
    ]);
    for (const entry of entries) {
      const line = insertLogLine(entry, new Date(0));
      expect(line).not.toContain('EPAS1');
      expect(JSON.parse(line)).toMatchObject({ at: '1970-01-01T00:00:00.000Z', chars: 13 });
    }
    expect(
      JSON.parse(
        insertLogLine({
          route: 'other_app',
          method: 'paste',
          bundle: 'not a "bundle"',
          result: 'inserted',
          chars: -1,
        })
      )
    ).toMatchObject({ bundle: null, chars: 0 });
  });
});

// F13 (DD-102): a start from fn or the global shortcut while the last dictation still transcribes
// waits, one deep, and begins the moment that transcript arrives.
describe('a start while the last dictation transcribes', () => {
  const transcribing = () =>
    nextAnywhereState(
      nextAnywhereState(nextAnywhereState(INITIAL_ANYWHERE_STATE, { type: 'enable' }), {
        type: 'start',
        source: 'shortcut',
        privateConfirmed: false,
      }),
      { type: 'stop' }
    );

  it('is queued, and begins once the transcript arrives', () => {
    const before = transcribing();
    expect(before).toMatchObject({ phase: 'transcribing', queued: null });
    const waiting = nextAnywhereState(before, {
      type: 'start',
      source: 'shortcut',
      privateConfirmed: false,
    });
    // The dictation that transcribes is left as it was; only the next one waits.
    expect(waiting).toEqual({ ...before, queued: { source: 'shortcut' } });
    // Main reads the queued start, finishes the last dictation, then starts the queued one.
    const queued = waiting.queued;
    const rest = nextAnywhereState(waiting, { type: 'finish' });
    expect(rest).toMatchObject({ phase: 'idle', queued: null });
    expect(queued).not.toBeNull();
    expect(
      nextAnywhereState(rest, { type: 'start', source: queued!.source, privateConfirmed: false })
    ).toMatchObject({ phase: 'arming', source: 'shortcut', includePrivateTerms: false });
  });

  it('keeps one: a second start during the wait is ignored', () => {
    const waiting = nextAnywhereState(transcribing(), {
      type: 'start',
      source: 'shortcut',
      privateConfirmed: false,
    });
    expect(
      nextAnywhereState(waiting, { type: 'start', source: 'shortcut', privateConfirmed: false })
    ).toBe(waiting);
  });

  it('is dropped by a stop or cancel for its gesture, and the last dictation goes on', () => {
    const before = transcribing();
    const waiting = nextAnywhereState(before, {
      type: 'start',
      source: 'shortcut',
      privateConfirmed: false,
    });
    expect(nextAnywhereState(waiting, { type: 'unqueue' })).toEqual(before);
    expect(nextAnywhereState(before, { type: 'unqueue' })).toBe(before);
    // Off, a cancel and a refusal end everything, the wait with it.
    expect(nextAnywhereState(waiting, { type: 'disable' })).toEqual(INITIAL_ANYWHERE_STATE);
    expect(nextAnywhereState(waiting, { type: 'cancel' }).queued).toBeNull();
    expect(
      nextAnywhereState(waiting, { type: 'refuse', reason: 'capture-failed' }).queued
    ).toBeNull();
  });

  it('never queues a tray or preview start, nor one while the switch is off', () => {
    const before = transcribing();
    for (const source of ['tray', 'preview'] as const) {
      expect(nextAnywhereState(before, { type: 'start', source, privateConfirmed: false })).toBe(
        before
      );
    }
    const off = { ...before, switchOn: false };
    expect(
      nextAnywhereState(off, { type: 'start', source: 'shortcut', privateConfirmed: false })
    ).toBe(off);
    // Nothing waits behind a dictation that is still arming or recording: a press there stops it.
    for (const phase of ['arming', 'recording'] as const) {
      const busy = { ...before, phase };
      expect(
        nextAnywhereState(busy, { type: 'start', source: 'shortcut', privateConfirmed: false })
          .queued
      ).toBeNull();
    }
  });
});

// F10 and F15 (DD-102): **Copy last dictation** holds the last text for its remaining time, in
// main's memory only, after Biorouter typed it into another app or a new start closed its card.
describe('the last dictation main holds', () => {
  it('holds typed text for REVIEW_TEXT_MS, and gives it back only before then', () => {
    const held = heldLastDictation('typed words', 1_000);
    expect(held).toEqual({ text: 'typed words', expiresAt: 1_000 + REVIEW_TEXT_MS });
    expect(lastDictationBeforeExpiry(held, 1_000 + REVIEW_TEXT_MS - 1)).toBe('typed words');
    expect(lastDictationBeforeExpiry(held, 1_000 + REVIEW_TEXT_MS)).toBeNull();
    expect(lastDictationBeforeExpiry(null, 0)).toBeNull();
  });

  it('is kept after a paste, never after a copy (the clipboard has it) or a card', () => {
    expect(holdsLastDictation({ kind: 'inserted' })).toBe(true);
    expect(holdsLastDictation({ kind: 'inserted', restored: 'cleared' })).toBe(true);
    for (const delivered of [
      { kind: 'copied' },
      { kind: 'copied', reason: 'secure-input' },
      { kind: 'review' },
      { kind: 'nothing' },
      { kind: 'echo' },
      { kind: 'old-backend' },
    ] as const) {
      expect(holdsLastDictation(delivered), delivered.kind).toBe(false);
    }
  });

  it('takes a closed review card’s text with the time it had left', () => {
    expect(reviewHandOff('card text', 9_000, 4_000)).toEqual({
      text: 'card text',
      expiresAt: 9_000,
    });
    expect(reviewHandOff('card text', 9_000, 9_000)).toBeNull();
    expect(reviewHandOff(null, 9_000, 4_000)).toBeNull();
    expect(reviewHandOff('card text', null, 4_000)).toBeNull();
  });
});

// F4, F5 and F8 (DD-102): what the paste did to the clipboard, said only when it was not all put back.
describe('how a paste left the clipboard', () => {
  const target: InsertTarget = { generation: 4, pid: 812, bundle: 'com.apple.TextEdit' };
  const delivery = (insert: AnywhereDelivery['insert']) => {
    let board = '';
    return {
      insert: vi.fn(insert),
      write: vi.fn((text: string) => {
        board = text;
      }),
      read: vi.fn(() => board),
      notify: vi.fn(),
      review: vi.fn(),
      pasteKey: '⌘V',
      oldBackendMessage: 'old daemon',
      log: vi.fn(),
    } satisfies AnywhereDelivery;
  };

  it('notifies for partial, kept-text and cleared, and nothing for all, moved or an older helper', async () => {
    for (const restored of [...INSERT_RESTORED, undefined]) {
      const deps = delivery(async () => (restored ? { ok: true, restored } : { ok: true }));
      const delivered = await deliverAnywhereTranscript(transcript(), target, deps);
      expect(delivered.kind, String(restored)).toBe('inserted');
      expect(deps.write).not.toHaveBeenCalled();
      expect(deps.review).not.toHaveBeenCalled();
      const message = insertRestoreMessage(restored);
      if (restored === 'partial' || restored === 'kept-text' || restored === 'cleared') {
        expect(message).not.toBeNull();
        expect(deps.notify, restored).toHaveBeenCalledExactlyOnceWith(message);
      } else {
        expect(deps.notify, String(restored)).not.toHaveBeenCalled();
      }
    }
    expect(insertRestoreMessage('partial')).toBe(dictationMessage('MSG-INSERT-CLIPBOARD-PARTIAL'));
    expect(insertRestoreMessage('kept-text')).toBe(dictationMessage('MSG-INSERT-CLIPBOARD-KEPT'));
    expect(insertRestoreMessage('cleared')).toBe(dictationMessage('MSG-INSERT-PASSWORD-CLEARED'));
  });

  it('records the outcome in the insert log, and never the text', async () => {
    const deps = delivery(async () => ({ ok: true, restored: 'partial' }));
    await deliverAnywhereTranscript(transcript(), target, deps);
    expect(deps.log).toHaveBeenCalledExactlyOnceWith({
      route: 'other_app',
      method: 'paste',
      bundle: 'com.apple.TextEdit',
      result: 'inserted',
      restored: 'partial',
      chars: 11,
    });
    const line = JSON.parse(insertLogLine(deps.log.mock.calls[0][0], new Date(0)));
    expect(line.restored).toBe('partial');
    expect(
      JSON.parse(
        insertLogLine({
          route: 'other_app',
          method: 'paste',
          bundle: null,
          result: 'inserted',
          restored: 'odd' as never,
          chars: 1,
        })
      )
    ).not.toHaveProperty('restored');
  });

  it('copies a transcript too long to send, without a line reaching the helper', async () => {
    const written: string[] = [];
    const service = new TextInsertService({
      platform: 'darwin',
      helperPath: '/h',
      spawn: () => ({
        stdout: { on: () => undefined },
        stdin: { write: (text: string) => written.push(text), end: () => undefined },
        on: () => undefined,
        kill: () => undefined,
      }),
    });
    service.start();
    const long = 'é'.repeat(MAX_REQUEST_LINE_BYTES / 2);
    const deps = delivery((pid, text) => service.insert(pid, text));
    expect(
      await deliverAnywhereTranscript(transcript({ text: long, raw_text: long }), target, deps)
    ).toEqual({ kind: 'copied', reason: 'failed' });
    expect(written).toEqual([]);
    expect(deps.write).toHaveBeenCalledExactlyOnceWith(long);
    expect(deps.notify).toHaveBeenCalledExactlyOnceWith(insertRefusalMessage('failed', '⌘V'));
    expect(insertRefusalMessage('failed', '⌘V')).toBe(
      dictationMessage('MSG-INSERT-FAILED', { key: '⌘V' })
    );
    service.stop();
  });
});

describe('telling every window the history changed (F18)', () => {
  it('sends at once, then no sooner than 250 ms after the last one', () => {
    expect(historyBroadcastDelay(-Infinity, 10, 250)).toBe(0);
    expect(historyBroadcastDelay(1_000, 1_100, 250)).toBe(150);
    expect(historyBroadcastDelay(1_000, 1_250, 250)).toBe(0);
    expect(historyBroadcastDelay(1_000, 9_000, 250)).toBe(0);
    // A clock that went backwards waits at most the interval.
    expect(historyBroadcastDelay(5_000, 1_000, 250)).toBe(250);
  });
});

describe('the capture window between dictations', () => {
  it('stays warm only while fn may dictate into other apps', () => {
    const warm = { platform: 'darwin', fnEnabled: true, fnOtherApps: true, anywhereOn: true };
    expect(captureWindowStaysWarm(warm)).toBe(true);
    expect(captureWindowStaysWarm({ ...warm, fnOtherApps: false })).toBe(false);
    expect(captureWindowStaysWarm({ ...warm, fnEnabled: false })).toBe(false);
    expect(captureWindowStaysWarm({ ...warm, anywhereOn: false })).toBe(false);
    expect(captureWindowStaysWarm({ ...warm, platform: 'win32' })).toBe(false);
  });

  it('a release while arming is a stop: the state goes to transcribing, never back to rest', () => {
    const arming = nextAnywhereState(
      nextAnywhereState(INITIAL_ANYWHERE_STATE, { type: 'enable' }),
      {
        type: 'start',
        source: 'shortcut',
        privateConfirmed: false,
      }
    );
    expect(arming.phase).toBe('arming');
    const stopped = nextAnywhereState(arming, { type: 'stop' });
    expect(stopped.phase).toBe('transcribing');
    // The microphone opening afterwards does not turn it back into a recording.
    expect(nextAnywhereState(stopped, { type: 'capture-ready' })).toBe(stopped);
  });
});

describe('the dictation test seams (DESIGN 3.12)', () => {
  const env = {
    [DICTATION_TEST_SEAM_ENV.fnTrace]: '1',
    [DICTATION_TEST_SEAM_ENV.clipboardFile]: '/tmp/run/clipboard.txt',
    [DICTATION_TEST_SEAM_ENV.insertLog]: '/tmp/run/insert.log',
  };

  it('a packaged app reads none of them, whatever its environment holds', () => {
    expect(dictationTestSeams({ isPackaged: true }, env)).toEqual({
      fnTrace: false,
      clipboardFile: null,
      insertLog: null,
    });
  });

  it('an unpackaged app reads each, and believes only an absolute path and a trace of 1', () => {
    expect(dictationTestSeams({ isPackaged: false }, env)).toEqual({
      fnTrace: true,
      clipboardFile: '/tmp/run/clipboard.txt',
      insertLog: '/tmp/run/insert.log',
    });
    expect(
      dictationTestSeams(
        { isPackaged: false },
        {
          [DICTATION_TEST_SEAM_ENV.fnTrace]: 'true',
          [DICTATION_TEST_SEAM_ENV.clipboardFile]: 'clipboard.txt',
          [DICTATION_TEST_SEAM_ENV.insertLog]: '/tmp/a\0b',
        }
      )
    ).toEqual({ fnTrace: false, clipboardFile: null, insertLog: null });
    expect(dictationTestSeams({ isPackaged: false }, {})).toEqual({
      fnTrace: false,
      clipboardFile: null,
      insertLog: null,
    });
  });
});
