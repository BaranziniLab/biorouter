import { describe, expect, it, vi } from 'vitest';
import type { Transcript } from '../components/dictation/dictationApi';
import {
  anywhereAtRest,
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

// Owner decision, 2026-11-10: Dictate into this app works whether or not Dictate into other
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
