/**
 * Which composer in this window owns the microphone, and one dictation from
 * the press to the text in the field ("Ownership and lifecycle" in
 * `docs/dictation/surfaces.md`).
 *
 * A per window module store, free of React, so the chat composer, the message
 * editor, Crew's boxes, the clipboard capture window and the preview bar all
 * drive dictation the same way. A composer attaches itself with
 * {@link attachDictationComposer} while it is mounted; a microphone calls
 * {@link startDictation} inside its click or key handler; the snapshot says what
 * to show.
 *
 * The snapshot changes only when what a composer shows changes: the activity, a
 * notice, the partial line, MSG-WAIT, or the whole second of the timer. The
 * microphone's level changes with every 100 ms chunk, so it lives outside the
 * snapshot ({@link getDictationLevel}, {@link subscribeDictationLevel}), and the
 * level bars read it in their animation frame without a React render.
 *
 * The path from the press to text, with nothing on it that could have run
 * earlier:
 *
 * 1. Press: capture starts inside the gesture (`capture.ts`), and the stream is
 *    created in the same moment when the status store already said the
 *    microphone is ready. Audio captured before the daemon answers is buffered,
 *    never dropped.
 * 2. While the person speaks, audio goes up every 2 s, so the daemon decodes
 *    closed segments before the stop (`transport.ts`). A stream that fails
 *    meanwhile ends the dictation at once, with its notice.
 * 3. Stop: the worklet's last partial chunk, the last `PUT`, then `finish` at
 *    once. The context closes in parallel and is not waited for.
 * 4. The text goes in at the anchor, as one undo step where the field still has
 *    focus (`utils/dictatedText.ts`), or into the stored draft when the composer
 *    is gone. The anchor follows the person's edits until then, also while the
 *    recording waits in the background or is kept for **Retry**.
 *
 * Who owns the microphone, and what happens to speech in each case:
 *
 * - Dictation starting in pane B while pane A records stops A, and A's text goes
 *   into A. If A fails meanwhile, its notice (with **Retry**, when it has one)
 *   shows in A's card once B has ended, unless B ended with a failure of its own.
 * - A composer that unmounts while arming, recording or transcribing stops the
 *   capture, finishes the stream, and its text goes into its stored draft at the
 *   mapped anchor. A composer that mounts again under the same key in the same
 *   commit (a remount) keeps the recording going. One that unmounts while the
 *   first use macOS prompt is open never opens the microphone.
 * - A screen lock (`dictation:lock-screen`) and a track that ends (a headset
 *   unplugged) stop and insert what was captured.
 * - 30 s before the 600 s cap the card says MSG-CAP-WARN; at the cap the
 *   dictation stops, inserts, and says MSG-CAP-STOPPED.
 * - While arming, recording or transcribing, the window has told the main
 *   process `dictation:busy`, so **Restart & update** can ask first.
 *
 * Two windows may each record; the daemon queues their streams.
 */
import { insertIntoComposerDraft, type DictationRecording } from '../../utils/composerDrafts';
import {
  insertDictatedText,
  mapAnchor,
  type Anchor,
  type InsertPath,
} from '../../utils/dictatedText';
import { isBrowserSurface } from '../../utils/surface';
import { prewarmCapture, startCapture, type Capture, type CaptureOptions } from './capture';
import {
  DICTATION_LABELS,
  dictationMessage,
  WHERE_THIS_COMPUTER,
  whereOnServer,
  type DictationMessageId,
} from './copy';
import type { StreamRequest, Transcript } from './dictationApi';
import {
  dictationDaemonPlace,
  ensureDictationStatus,
  getDictationStatusSnapshot,
  readDictationViewer,
  refreshDictationStatus,
  remoteOldValues,
  resolveDictationPhase,
  type DictationStatusSnapshot,
  type PhaseDecision,
} from './dictationStore';
import { micErrorNotice } from './micErrors';
import { markPrivateDraft } from './privateDraft';
import {
  createDictationStream,
  DictationStreamError,
  type DictationStream,
  type StreamOptions,
} from './transport';

/** Under this many seconds of audio nothing is transcribed (MSG-TOO-SHORT). */
export const MIN_DICTATION_SECONDS = 0.5;
/** "3 s below the noise floor while recording" (MSG-NO-AUDIO-YET): -60 dBFS. */
export const NO_AUDIO_RMS = 0.001;
export const NO_AUDIO_AFTER_S = 3;
/** A recording whose transcription failed is kept this long for **Retry**. */
export const KEEP_FAILED_MS = 5 * 60 * 1000;
/** How long an insert waits for an IME composition to end before taking the state path. */
export const COMPOSITION_WAIT_MS = 3_000;

export type DictationActivity = 'idle' | 'arming' | 'recording' | 'transcribing' | 'error';

export type DictationNoticeAction = 'retry' | 'open-settings' | null;

export interface DictationNotice {
  /** The catalog id, or `server` for a sentence the daemon wrote (a policy refusal). */
  id: DictationMessageId | 'server';
  text: string;
  action: DictationNoticeAction;
  /** Whether it reports a failure (the `error` look) or a result such as MSG-NOTHING. */
  failure: boolean;
}

export interface DictationSnapshot {
  /** The composer that owns the microphone, or that the last notice belongs to. */
  key: string | null;
  activity: DictationActivity;
  /**
   * Seconds of audio captured so far, published once per whole second for the
   * m:ss timer. The level is not here: see {@link getDictationLevel}.
   */
  elapsedS: number;
  /** The muted preview line; never in the field (DD-31). */
  partial: string;
  /** MSG-WAIT. */
  waiting: boolean;
  notice: DictationNotice | null;
  /** The text the hidden `role="status"` region announces. */
  announcement: string;
  /**
   * Set when the daemon answered 409 `noModel` although the status said the
   * microphone was ready: the composer named here opens the setup popover.
   * A new object each time, so a watcher can tell two requests apart.
   */
  setupRequest: { key: string } | null;
}

const IDLE: DictationSnapshot = {
  key: null,
  activity: 'idle',
  elapsedS: 0,
  partial: '',
  waiting: false,
  notice: null,
  announcement: '',
  setupRequest: null,
};

/** What a mounted composer lends the session. */
export interface DictationComposer {
  /** The composer's draft key (`utils/composerDrafts.ts`), or another key unique in this window. */
  key: string;
  /** The field, while it is mounted. */
  field(): HTMLTextAreaElement | null;
  /** The composer's state path (DD-63 step 4). */
  setByState(value: string, caret: number): void;
  /** Whether an IME composition is running in the field. */
  isComposing?(): boolean;
  /**
   * Whether `key` is a stored draft that receives the text after this composer
   * unmounts. Composers without one (the message editor, Crew's boxes) drop it.
   */
  storesDraft: boolean;
  /**
   * How many characters the stored draft holds before the field's own text (the
   * chat composer's `/bug ` command chip); 0 when absent. Read while the
   * composer is mounted, and remembered for when the text arrives after it is
   * gone.
   */
  draftPrefix?(): number;
  /** After the text went in: the composer marks the box typed and records the span. */
  onInserted?(event: DictationInserted): void;
}

export interface DictationInserted {
  key: string;
  path: InsertPath;
  transcript: Transcript;
  /** The anchor the text went to, as mapped at that moment. */
  anchor: Anchor;
  streamId: string | null;
  /** The field before insertion, so an event render cannot replace the source. */
  before: string;
}

export type DictationTarget = Omit<StreamRequest, 'sample_rate' | 'format'>;

export interface StartDictationOptions {
  key: string;
  target: DictationTarget;
  /** The selection to replace; defaults to the field's selection now. */
  anchor?: Anchor;
}

/** What a press led to: recording, the setup popover, or a reason. */
export type StartOutcome =
  | { kind: 'recording' }
  | { kind: 'stopping' }
  | { kind: 'setup' }
  | { kind: 'unavailable'; reason: string }
  | { kind: 'removed' }
  | { kind: 'failed'; notice: DictationNotice }
  | { kind: 'cancelled' };

interface Deps {
  startCapture(options: CaptureOptions): Promise<Capture>;
  /** Prepares the next press (`capture.ts`); a no-op where nothing can be prepared. */
  prewarmCapture(): void;
  createStream(request: StreamRequest, options?: StreamOptions): DictationStream;
  statusSnapshot(): DictationStatusSnapshot;
  ensureStatus(): Promise<unknown>;
  refreshStatus(): Promise<unknown>;
  phase(): PhaseDecision;
  insertIntoDraft(key: string, at: Anchor, text: string, recording?: DictationRecording): void;
}

const DEFAULT_DEPS: Deps = {
  startCapture,
  prewarmCapture,
  createStream: createDictationStream,
  statusSnapshot: getDictationStatusSnapshot,
  ensureStatus: ensureDictationStatus,
  refreshStatus: refreshDictationStatus,
  // No environment passed: the store fills the page's own address, the
  // daemon's and this app's version into the sentences it returns.
  phase: () => resolveDictationPhase(getDictationStatusSnapshot()),
  insertIntoDraft: insertIntoComposerDraft,
};

let deps: Deps = DEFAULT_DEPS;
let snapshot: DictationSnapshot = IDLE;
const listeners = new Set<() => void>();
let level = 0;
const levelListeners = new Set<(level: number) => void>();
const composers = new Map<string, DictationComposer>();
let active: Dictation | null = null;
/**
 * Every dictation that holds the microphone or still waits for its text: the
 * current one, and one a press in another pane displaced, which finishes into
 * its own composer in the background. The window is busy while any is here.
 */
const running = new Set<Dictation>();
/** A failed dictation's audio, kept for **Retry**. */
let kept: {
  key: string;
  stream: DictationStream;
  dictation: Dictation;
  timer: ReturnType<typeof setTimeout>;
} | null = null;
/** A displaced dictation's failure, waiting for the card to be free. */
let pendingFailure: { key: string; notice: DictationNotice } | null = null;
let lockScreenInstalled = false;
/** The composer whose field the text is going into right now (see {@link isDictationInserting}). */
let inserting: string | null = null;
/** Bumped by the test reset, so a preparation scheduled before it does nothing after it. */
let sessionEra = 0;
let busyReported = false;
const busyListeners = new Set<() => void>();

function publish(next: Partial<DictationSnapshot>): void {
  snapshot = { ...snapshot, ...next };
  if (snapshot.activity !== 'recording') setLevel(0);
  reportBusy();
  for (const listener of [...listeners]) listener();
}

export function getDictationSnapshot(): DictationSnapshot {
  return snapshot;
}

/**
 * Whether dictated text is going into `key`'s field at this moment, so the
 * composer's change handler can tell the insert from typing: a spoken "@" or
 * "/" opens no menu (DD-63 step 5), and the edit moves no anchor.
 */
export function isDictationInserting(key?: string): boolean {
  return inserting !== null && (key === undefined || inserting === key);
}

/**
 * What one composer shows of the session, reduced to its activity, so a large
 * composer can subscribe for its submit gates and render only when this
 * changes, never on the timer, the partial line or a notice.
 */
export function dictationActivityFor(key: string): DictationActivity {
  return snapshot.key === key ? snapshot.activity : 'idle';
}

export function subscribeDictation(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/** The last 100 ms chunk's RMS, 0 to 1, of the recording the snapshot names; 0 otherwise. */
export function getDictationLevel(): number {
  return level;
}

/** Every new level, about 10 times a second while recording; never a React render. */
export function subscribeDictationLevel(listener: (level: number) => void): () => void {
  levelListeners.add(listener);
  return () => levelListeners.delete(listener);
}

function setLevel(next: number): void {
  if (next === level) return;
  level = next;
  for (const listener of [...levelListeners]) {
    try {
      listener(level);
    } catch {
      // A meter's failure is its own.
    }
  }
}

function reportBusy(): void {
  const busy = running.size > 0;
  if (busy === busyReported) return;
  busyReported = busy;
  try {
    window.electron?.setDictationBusy?.(busy);
  } catch {
    // A surface without the bridge has no restart to guard.
  }
  for (const listener of [...busyListeners]) listener();
}

/**
 * Whether any dictation in this window is arming, recording or transcribing, a displaced one
 * finishing in the background included: what main's busy record holds for this window. Settings
 * holds **Test speed** while it is true, because a bench decodes on the same worker as the
 * dictation and would sit on its path from stop to text.
 */
export function isDictationRunning(): boolean {
  return busyReported;
}

/** Called each time {@link isDictationRunning} changes; never a React render by itself. */
export function subscribeDictationRunning(listener: () => void): () => void {
  busyListeners.add(listener);
  return () => busyListeners.delete(listener);
}

function installLockScreen(): void {
  if (lockScreenInstalled || typeof window === 'undefined') return;
  lockScreenInstalled = true;
  try {
    window.electron?.onDictationLockScreen?.(() => active?.stop());
  } catch {
    // A browser surface has no screen lock signal.
  }
}

// ── composers ───────────────────────────────────────────────────────────────

/**
 * A composer is mounted under `composer.key`. The returned function detaches it;
 * if no composer attaches under the same key before the next task, a dictation
 * it owned stops and its text goes to the stored draft, and a recording kept for
 * its **Retry** is dropped.
 */
export function attachDictationComposer(composer: DictationComposer): () => void {
  composers.set(composer.key, composer);
  return () => {
    if (composers.get(composer.key) !== composer) return;
    composers.delete(composer.key);
    setTimeout(() => {
      if (composers.has(composer.key)) return;
      if (active && active.key === composer.key) active.stop();
      if (kept?.key === composer.key) forgetKept();
      if (pendingFailure?.key === composer.key) pendingFailure = null;
    }, 0);
  };
}

/**
 * The composer's prose changed: the anchor of every dictation that will still
 * put text into it moves with the edit. That is the current one, one a press in
 * another pane displaced, and one kept for **Retry**.
 */
export function noteDictationEdit(
  key: string,
  before: string,
  after: string,
  caretAfter?: number
): void {
  const affected = new Set<Dictation>();
  for (const dictation of running) if (dictation.key === key) affected.add(dictation);
  if (kept?.key === key) affected.add(kept.dictation);
  const prefix = composers.get(key)?.draftPrefix?.();
  for (const dictation of affected) {
    dictation.mapAnchor(before, after, caretAfter);
    if (prefix !== undefined) dictation.draftPrefix = prefix;
  }
}

// ── notices ─────────────────────────────────────────────────────────────────

const notice = (
  id: DictationMessageId,
  failure: boolean,
  action: DictationNoticeAction = null,
  values: Record<string, string> = {}
): DictationNotice => ({ id, text: dictationMessage(id, values), action, failure });

/** MSG-ENGINE-FAILED's `<where>`, from where the daemon is, never from the surface alone (DD-92). */
function whereTranscribed(): string {
  const place = dictationDaemonPlace(deps.statusSnapshot().status);
  return place.kind === 'this-computer' ? WHERE_THIS_COMPUTER : whereOnServer(place.host);
}

function failureNotice(error: unknown): DictationNotice {
  if (!(error instanceof DictationStreamError)) return notice('MSG-LOST', true);
  switch (error.code) {
    case 'engineFailed':
      return notice('MSG-ENGINE-FAILED', true, 'retry', { where: whereTranscribed() });
    case 'proxy413':
      return notice('MSG-413', true);
    case 'tooManyStreams':
      // MSG-SERVER-BUSY promises another try; this one says the waiting is over.
      return notice('MSG-SERVER-BUSY-FAILED', true, 'retry');
    case 'absent':
      return notice('MSG-REMOTE-OLD', true, null, remoteOldValues());
    case 'policy':
    case 'unavailable':
    case 'serverMemory':
      if (error.serverMessage) {
        return { id: 'server', text: error.serverMessage, action: null, failure: true };
      }
      return notice(error.code === 'policy' ? 'MSG-POLICY' : 'MSG-LOST', true);
    default:
      return notice('MSG-LOST', true);
  }
}

/**
 * Whether a displaced dictation's failure can still reach its card: not when
 * nothing records and the card already shows another composer's failure, which
 * stays (with its own kept recording).
 */
function cardTakesDisplacedFailure(key: string): boolean {
  return !(active === null && snapshot.notice?.failure === true && snapshot.key !== key);
}

/**
 * A dictation that a press in another pane displaced ended with a failure. It
 * shows in its own card as soon as the card is free; while another dictation
 * holds the card it waits for that one to end.
 */
function noteDisplacedFailure(key: string, failure: DictationNotice): void {
  if (!composers.has(key) || !cardTakesDisplacedFailure(key)) return;
  pendingFailure = { key, notice: failure };
  if (active === null) showPendingFailure();
}

function showPendingFailure(): void {
  const pending = pendingFailure;
  pendingFailure = null;
  if (!pending) return;
  publish({
    ...IDLE,
    key: pending.key,
    activity: 'error',
    notice: pending.notice,
    announcement: pending.notice.text,
  });
}

/** The current dictation ended with a failure of its own, which keeps the card. */
function dropPendingFailure(owner: Dictation): void {
  const pending = pendingFailure;
  pendingFailure = null;
  if (pending && kept?.key === pending.key && kept.dictation !== owner) dropKept();
}

// ── one dictation ───────────────────────────────────────────────────────────

/** A letter, a digit or a combining mark: what a word is made of. */
const WORD_LETTER = /[\p{L}\p{N}\p{M}]/u;
/** Joins two parts of one word ("don't", "HIF-2α", "snake_case") when letters are on both sides. */
const WORD_JOINER = /['’_-]/u;

/**
 * A caret strictly inside a word, between two letters, moves to the nearer end
 * of that word (its end on a tie), so the text goes in beside the word and
 * never splits it: "Call th|e OMOP" takes the dictation after "the". A click
 * lands between two letters far more often than a person means to put words
 * there. A selection, and a caret at a word's edge, stay where they are.
 */
export function anchorOutsideWord(value: string, anchor: Anchor): Anchor {
  const at = anchor.start;
  if (at !== anchor.end || at <= 0 || at >= value.length) return anchor;
  if (!WORD_LETTER.test(value[at - 1]) || !WORD_LETTER.test(value[at])) return anchor;
  // A joiner counts only with a letter beyond it, so "a -b" is two words.
  const inWord = (i: number, beyond: number) =>
    WORD_LETTER.test(value[i]) ||
    (WORD_JOINER.test(value[i]) && WORD_LETTER.test(value[beyond] ?? ''));
  let start = at;
  while (start > 0 && inWord(start - 1, start - 2)) start--;
  let end = at;
  while (end < value.length && inWord(end, end + 1)) end++;
  const moved = at - start < end - at ? start : end;
  return { start: moved, end: moved };
}

class Dictation {
  readonly key: string;
  private anchor: Anchor;
  private capture: Capture | null = null;
  private captureSettled = false;
  private readonly captureWaiters: Array<(capture: Capture | null) => void> = [];
  private stream: DictationStream | null = null;
  /** Audio captured while no stream exists yet; pushed when it opens. */
  private pending: Int16Array[] = [];
  private samples = 0;
  private rate = 16000;
  private stopping = false;
  private cancelled = false;
  /** The dictation has ended one way or another; no stream may open after this. */
  private finished = false;
  private heardAnything = false;
  private loudAtS = 0;
  private quietNoticeShown = false;
  private capStopped = false;
  /** The stop's `finish`, when it was started before the card was told (see `finish`). */
  private finishing: Promise<Transcript> | null = null;
  private busyTimer: ReturnType<typeof setInterval> | null = null;
  private readonly unsubscribe: Array<() => void> = [];

  /**
   * Whether the composer that started this dictation keeps a stored draft under
   * its key, remembered for when it has unmounted by the time the text arrives.
   * A key no composer was attached under gets no draft written.
   */
  private readonly storesDraft: boolean;
  /** The composer's {@link DictationComposer.draftPrefix}, as last read. */
  draftPrefix: number;

  constructor(private readonly options: StartDictationOptions) {
    this.key = options.key;
    this.storesDraft = composers.get(options.key)?.storesDraft ?? false;
    this.draftPrefix = composers.get(options.key)?.draftPrefix?.() ?? 0;
    const field = composers.get(options.key)?.field() ?? null;
    const end = field?.value.length ?? 0;
    const anchor = options.anchor ?? {
      start: field?.selectionStart ?? end,
      end: field?.selectionEnd ?? end,
    };
    this.anchor = field ? anchorOutsideWord(field.value, anchor) : anchor;
  }

  private isCurrent(): boolean {
    return active === this;
  }

  /** A stop was asked for (a press, an unmount, a screen lock). */
  isStopping(): boolean {
    return this.stopping;
  }

  mapAnchor(before: string, after: string, caretAfter?: number): void {
    this.anchor = mapAnchor(this.anchor, before, after, caretAfter);
  }

  /** Capture's steps 1 to 3 run synchronously inside this call. */
  begin(openStreamNow: boolean): Promise<Capture> {
    const viewer = readDictationViewer();
    const capturePromise = deps.startCapture({
      deviceId: viewer.deviceId,
      deviceLabel: viewer.deviceLabel,
      noiseSuppression: viewer.noiseReduction,
      onChunk: (pcm, chunkLevel) => this.onChunk(pcm, chunkLevel.rms, chunkLevel.peak),
      onLive: () => {
        if (this.isCurrent() && snapshot.activity === 'arming') {
          publish({ activity: 'recording', announcement: DICTATION_LABELS.announceRecording });
        }
      },
    });
    if (openStreamNow) this.openStream(16000);
    return capturePromise.then(
      (capture) => {
        this.capture = capture;
        this.rate = capture.sampleRate;
        if (this.cancelled) {
          capture.cancel();
        } else {
          // The Firefox fallback runs at the device's own rate, which the stream
          // must declare; one opened early for 16 kHz is replaced before any
          // audio went up.
          if (this.stream && capture.sampleRate !== 16000) {
            this.dropStream();
            this.openStream(capture.sampleRate);
          }
          void capture.ended.then((why) => {
            if (why === 'track-ended') this.stop();
          });
        }
        this.settleCapture(capture);
        return capture;
      },
      (error: unknown) => {
        this.settleCapture(null);
        throw error;
      }
    );
  }

  private settleCapture(capture: Capture | null): void {
    this.captureSettled = true;
    for (const waiter of this.captureWaiters.splice(0)) waiter(capture);
  }

  private captureOnceSettled(): Promise<Capture | null> {
    if (this.captureSettled) return Promise.resolve(this.capture);
    return new Promise((resolve) => this.captureWaiters.push(resolve));
  }

  openStream(sampleRate: number): void {
    if (this.cancelled || this.finished || this.stream) return;
    const stream = deps.createStream({
      ...this.options.target,
      sample_rate: sampleRate,
      format: 's16le',
    });
    this.stream = stream;
    this.listen(stream);
    stream.ready.catch((error: unknown) => {
      if (!this.cancelled && this.stream === stream) this.fail(error);
    });
    for (const pcm of this.pending.splice(0)) stream.push(pcm);
  }

  /** What the stream reports reaches the card while this dictation is current. */
  private listen(stream: DictationStream): void {
    this.unsubscribe.push(
      stream.onPartial((partial) => {
        if (this.isCurrent()) publish({ partial });
      }),
      stream.onWaiting((waiting) => {
        if (this.isCurrent()) publish({ waiting });
      }),
      stream.onCapWarning(() => {
        if (this.isCurrent()) publish({ notice: notice('MSG-CAP-WARN', false) });
      }),
      stream.onCapReached(() => {
        this.capStopped = true;
        this.stop();
      }),
      stream.onBusy((seconds) => this.countDownBusy(seconds)),
      // A stream that failed while the person still speaks ends the dictation
      // now: the microphone goes off and the notice shows without a stop.
      stream.onFailed((error) => {
        if (!this.cancelled && this.stream === stream) this.fail(error);
      })
    );
  }

  private dropStream(): void {
    const stream = this.stream;
    this.stream = null;
    for (const off of this.unsubscribe.splice(0)) off();
    void stream?.cancel();
  }

  /** MSG-SERVER-BUSY, counted down to the next try; a new 429 starts it again. */
  private countDownBusy(seconds: number): void {
    this.stopBusyCountdown();
    let left = seconds;
    const show = () => {
      if (this.isCurrent()) {
        publish({ notice: notice('MSG-SERVER-BUSY', false, null, { n: String(left) }) });
      }
    };
    show();
    this.busyTimer = setInterval(() => {
      left -= 1;
      if (left <= 0 || !this.isCurrent()) {
        this.stopBusyCountdown();
        if (this.isCurrent() && snapshot.notice?.id === 'MSG-SERVER-BUSY') {
          publish({ notice: null });
        }
        return;
      }
      show();
    }, 1000);
  }

  private stopBusyCountdown(): void {
    if (this.busyTimer === null) return;
    clearInterval(this.busyTimer);
    this.busyTimer = null;
  }

  private onChunk(pcm: Int16Array, rms: number, peak: number): void {
    if (this.cancelled) return;
    if (peak > 0) this.heardAnything = true;
    this.samples += pcm.length;
    if (this.stream) this.stream.push(pcm);
    else this.pending.push(pcm);
    // The worklet's last partial chunk arrives after the stop: it goes up with
    // the rest, but the card already shows Transcribing, with no level.
    if (!this.isCurrent() || this.stopping) return;
    const elapsedS = this.samples / this.rate;
    // Only what a composer shows: a render per chunk would add input lag to a
    // person typing while dictating.
    const next: Partial<DictationSnapshot> = {};
    if (snapshot.activity === 'arming') {
      next.activity = 'recording';
      next.announcement = DICTATION_LABELS.announceRecording;
    }
    if (Math.floor(elapsedS) !== Math.floor(snapshot.elapsedS)) next.elapsedS = elapsedS;
    if (rms >= NO_AUDIO_RMS) {
      this.loudAtS = elapsedS;
      if (snapshot.notice?.id === 'MSG-NO-AUDIO-YET') next.notice = null;
    } else if (!this.quietNoticeShown && elapsedS - this.loudAtS >= NO_AUDIO_AFTER_S) {
      this.quietNoticeShown = true;
      next.notice = notice('MSG-NO-AUDIO-YET', false);
    }
    if (Object.keys(next).length > 0) publish(next);
    setLevel(rms);
  }

  /** Stop and insert. A no-op once stopping or cancelled. */
  stop(): void {
    if (this.stopping || this.cancelled) return;
    this.stopping = true;
    void this.finish();
  }

  /** Nothing is inserted. */
  cancel(): void {
    if (this.cancelled) return;
    this.cancelled = true;
    this.finished = true;
    this.pending = [];
    this.stopBusyCountdown();
    this.capture?.cancel();
    this.dropStream();
    this.releaseWaiters();
    running.delete(this);
    if (this.isCurrent()) {
      active = null;
      publish({ ...IDLE, key: this.key });
      showPendingFailure();
    } else {
      reportBusy();
    }
    this.prepareNext();
  }

  /**
   * The person may dictate again into the same composer with nothing in
   * between that notes interest (the field keeps its focus, the pointer stays
   * on the microphone), so the next press is prepared once this capture's
   * context has closed, after the text went in and the card's render, off
   * every path.
   */
  private prepareNext(): void {
    if (!composers.has(this.key)) return;
    const { prewarmCapture: prepare } = deps;
    const era = sessionEra;
    void (this.capture?.closed ?? Promise.resolve()).then(() => {
      setTimeout(() => {
        if (era === sessionEra && composers.has(this.key)) prepare();
      }, 0);
    });
  }

  /**
   * A stop that is waiting for a capture or a stream that will now never come
   * (a cancel before the macOS prompt was answered, a failure) goes on, and
   * finds the dictation ended.
   */
  private releaseWaiters(): void {
    for (const waiter of this.streamWaiters.splice(0)) waiter();
    if (!this.captureSettled) {
      for (const waiter of this.captureWaiters.splice(0)) waiter(null);
    }
  }

  private async finish(): Promise<void> {
    // The worklet's capture usually hands over its last audio inside `stop()`, so
    // when the stream is open the stop's `finish` is started before the card is
    // told: the request is on the wire (`dictationApi.ts` sends it in the first
    // microtask) before the Transcribing render the publish below schedules.
    // The checks are the ones the path below makes, which then gets the same
    // `finish` back. A stop that waits for audio recorded before it, still queued
    // behind a busy main thread, has not handed it over yet: `finish` waits for
    // `stop()` below, or that audio would never reach the transcript.
    if (this.captureSettled && this.capture?.mode === 'worklet') {
      void this.capture.stop();
      if (
        this.capture.lastChunkDelivered &&
        this.stream &&
        !this.cancelled &&
        this.heardAnything &&
        this.samples / this.rate >= MIN_DICTATION_SECONDS
      ) {
        this.finishing = this.stream.finish();
        this.finishing.catch(() => undefined);
      }
    }
    if (this.isCurrent()) {
      publish({
        activity: 'transcribing',
        announcement: DICTATION_LABELS.announceTranscribing,
      });
    }
    // The capture may still be starting; its last partial chunk arrives before stop() settles.
    const capture = await this.captureOnceSettled();
    const captured = capture
      ? await capture.stop().then(
          () => true,
          () => false
        )
      : true;
    if (this.cancelled) return;
    if (!captured && capture?.mode === 'recorder') {
      // The MediaRecorder fallback delivers its audio only on stop, after
      // decoding it; when that fails, the length counted so far says nothing.
      this.dropStream();
      this.done(notice('MSG-RECORDING-FAILED', true));
      return;
    }
    if (this.samples / this.rate < MIN_DICTATION_SECONDS) {
      this.dropStream();
      this.done(notice('MSG-TOO-SHORT', false));
      return;
    }
    if (!this.heardAnything) {
      this.dropStream();
      this.done(notice('MSG-SILENT', false));
      return;
    }
    if (!this.stream) {
      // The status store had not loaded at the press; `startDictation` opens the
      // stream once it has, and pushes what was captured meanwhile.
      await this.streamOpened();
    }
    const stream = this.stream;
    if (!stream || this.cancelled) return;
    try {
      const early = this.finishing;
      this.finishing = null;
      const transcript = await (early ?? stream.finish());
      if (!this.cancelled) await this.deliver(transcript, stream);
    } catch (error) {
      if (!this.cancelled) this.fail(error);
    }
  }

  private readonly streamWaiters: Array<() => void> = [];
  private streamOpened(): Promise<void> {
    if (this.stream || this.cancelled) return Promise.resolve();
    return new Promise((resolve) => this.streamWaiters.push(resolve));
  }

  /** `startDictation` decided after the status loaded: open the stream, or not. */
  resolveStream(sampleRate: number | null): void {
    if (sampleRate !== null) this.openStream(sampleRate);
    for (const waiter of this.streamWaiters.splice(0)) waiter();
  }

  private async deliver(transcript: Transcript, stream: DictationStream): Promise<void> {
    if (transcript.no_speech || !transcript.text.trim()) {
      stream.release();
      this.done(notice('MSG-NOTHING', false));
      return;
    }
    let composer = composers.get(this.key);
    let field = composer?.field() ?? null;
    if (composer?.isComposing?.() && field) {
      await waitForCompositionEnd(field);
      // Escape during the wait: the person cancelled, and nothing goes in.
      if (this.cancelled) return;
      composer = composers.get(this.key);
      field = composer?.field() ?? null;
    }
    const anchor = this.anchor;
    const before = field?.value ?? '';
    let path: InsertPath;
    if (composer && field) {
      inserting = this.key;
      try {
        path = insertDictatedText(
          field,
          anchor,
          transcript.text,
          (value, caret) => composer!.setByState(value, caret),
          { composing: composer.isComposing?.() ?? false }
        );
      } finally {
        inserting = null;
      }
    } else {
      path = 'draft';
    }
    if (path === 'draft' && (composer?.storesDraft ?? this.storesDraft)) {
      // The anchor counts in the field's text; the stored draft also holds what
      // the field does not show before it (a `/bug ` command chip).
      const prefix = composer?.draftPrefix?.() ?? this.draftPrefix;
      deps.insertIntoDraft(
        this.key,
        { start: anchor.start + prefix, end: anchor.end + prefix },
        transcript.text,
        {
          recordId: stream.id ?? crypto.randomUUID(),
          raw: transcript.raw_text,
          changes: transcript.changes,
          model: transcript.model,
          biasScope: transcript.bias_scope,
        }
      );
      // The private draft flag (DD-71) is set here, not only by the composer's
      // `onInserted`: a composer that unmounted while the person spoke (a tab or
      // chat switch) is not in the map, and the draft it comes back to must still
      // ask before it leaves for a public model.
      if (transcript.bias_scope === 'private') markPrivateDraft(this.key);
    }
    composer?.onInserted?.({
      key: this.key,
      path,
      transcript,
      anchor,
      streamId: stream.id,
      before,
    });
    stream.release();
    this.done(
      this.capStopped ? notice('MSG-CAP-STOPPED', false) : null,
      DICTATION_LABELS.announceInserted
    );
  }

  private fail(error: unknown): void {
    if (this.cancelled) return;
    this.cancelled = true;
    this.finished = true;
    this.pending = [];
    this.capture?.cancel();
    const stream = this.stream;
    this.stream = null;
    for (const off of this.unsubscribe.splice(0)) off();
    this.releaseWaiters();
    if (error instanceof DictationStreamError && error.code === 'noModel') {
      this.stopBusyCountdown();
      void stream?.cancel();
      void deps.refreshStatus();
      running.delete(this);
      if (this.isCurrent()) {
        active = null;
        publish({ ...IDLE, key: this.key, setupRequest: { key: this.key } });
      } else {
        reportBusy();
      }
      return;
    }
    let failure = failureNotice(error);
    // The audio is kept only where a Retry can be shown for it: this dictation's
    // card, now or once another pane's dictation has ended.
    const shown =
      this.isCurrent() || (composers.has(this.key) && cardTakesDisplacedFailure(this.key));
    if (failure.action === 'retry' && stream && shown) {
      keepForRetry(this, stream);
    } else {
      void stream?.cancel();
      if (failure.action === 'retry') failure = { ...failure, action: null };
    }
    this.done(failure);
  }

  private done(result: DictationNotice | null, announcement?: string): void {
    this.finished = true;
    this.stopBusyCountdown();
    for (const off of this.unsubscribe.splice(0)) off();
    running.delete(this);
    this.prepareNext();
    if (!this.isCurrent()) {
      if (result?.failure) noteDisplacedFailure(this.key, result);
      reportBusy();
      return;
    }
    active = null;
    publish({
      ...IDLE,
      key: this.key,
      activity: result?.failure ? 'error' : 'idle',
      notice: result,
      announcement: announcement ?? result?.text ?? '',
    });
    if (result?.failure) dropPendingFailure(this);
    else showPendingFailure();
  }

  /** **Retry** after a retryable failure: the kept audio into a new stream. */
  async retry(stream: DictationStream): Promise<void> {
    this.cancelled = false;
    this.finished = false;
    this.stream = stream;
    // MSG-WAIT, and MSG-SERVER-BUSY if the new stream meets a 429, show again.
    this.listen(stream);
    running.add(this);
    publish({
      ...IDLE,
      key: this.key,
      activity: 'transcribing',
      announcement: DICTATION_LABELS.announceTranscribing,
    });
    try {
      const transcript = await stream.retry();
      if (!this.cancelled) await this.deliver(transcript, stream);
    } catch (error) {
      if (!this.cancelled) this.fail(error);
    }
  }
}

function waitForCompositionEnd(field: HTMLTextAreaElement): Promise<void> {
  return new Promise((resolve) => {
    const done = () => {
      clearTimeout(timer);
      field.removeEventListener('compositionend', done);
      resolve();
    };
    const timer = setTimeout(done, COMPOSITION_WAIT_MS);
    field.addEventListener('compositionend', done, { once: true });
  });
}

function keepForRetry(dictation: Dictation, stream: DictationStream): void {
  dropKept();
  const timer = setTimeout(() => {
    if (kept?.stream === stream) forgetKept();
  }, KEEP_FAILED_MS);
  kept = { key: dictation.key, stream, dictation, timer };
}

function dropKept(): void {
  if (!kept) return;
  clearTimeout(kept.timer);
  void kept.stream.cancel();
  kept = null;
}

/** Drop the kept recording, and take **Retry** off the notice that offered it. */
function forgetKept(): void {
  const key = kept?.key;
  dropKept();
  if (key !== undefined && snapshot.key === key && snapshot.notice?.action === 'retry') {
    publish({ notice: { ...snapshot.notice, action: null } });
  }
  if (key !== undefined && pendingFailure?.key === key && pendingFailure.notice.action) {
    pendingFailure = { key, notice: { ...pendingFailure.notice, action: null } };
  }
}

function outcomeFor(decision: PhaseDecision): StartOutcome {
  switch (decision.phase) {
    case 'not-installed':
    case 'installing':
      return { kind: 'setup' };
    case 'removed':
      return { kind: 'removed' };
    default:
      return { kind: 'unavailable', reason: decision.reason ?? '' };
  }
}

/**
 * Start dictating into the composer `options.key`. Call inside the click or key
 * handler: capture starts before this returns, and so does the stream when the
 * status store already said the microphone is ready. Before the store has
 * loaded, the stream waits for its one answer; if that answer is not `idle`,
 * capture stops and the outcome says why (the setup popover, or a reason).
 *
 * A press on the composer that is recording stops it and inserts; a press while
 * it is still arming cancels, as the phase table says, because nothing has been
 * heard yet. A press in another pane stops the pane that records only when this
 * one can start: a press that leads to the setup popover or a reason leaves it
 * recording.
 */
export function startDictation(options: StartDictationOptions): Promise<StartOutcome> {
  installLockScreen();
  if (active && active.key === options.key) {
    if (snapshot.key === options.key && snapshot.activity === 'arming') {
      active.cancel();
      return Promise.resolve({ kind: 'cancelled' });
    }
    active.stop();
    return Promise.resolve({ kind: 'stopping' });
  }

  const before = deps.phase();
  if (before.phase !== 'idle') return Promise.resolve(outcomeFor(before));

  if (active) {
    // Another pane records: it stops, and its text goes into it.
    const other = active;
    active = null;
    other.stop();
  }
  dropKept();
  pendingFailure = null;

  const statusLoaded = deps.statusSnapshot().load === 'loaded';
  const dictation = new Dictation(options);
  active = dictation;
  running.add(dictation);
  publish({ ...IDLE, key: options.key, activity: 'arming' });

  const askFirst =
    !isBrowserSurface() &&
    window.electron?.platform === 'darwin' &&
    deps.statusSnapshot().micStatus === 'not-determined' &&
    typeof window.electron?.dictationMicRequest === 'function';

  const run = async (): Promise<StartOutcome> => {
    if (askFirst) {
      // The first use on macOS: the prompt comes from the person's click and names Biorouter.
      const granted = await window.electron.dictationMicRequest!().catch(() => false);
      if (!granted) {
        const failed = notice('MSG-MIC-DENIED-MAC', true, 'open-settings');
        dictation.cancel();
        if (snapshot.key === options.key) publish({ activity: 'error', notice: failed });
        return { kind: 'failed', notice: failed };
      }
      // Cancelled, displaced by another pane, or stopped (its composer
      // unmounted, the screen locked) while the prompt was open: the microphone
      // is never opened for it, and a stop waiting on it ends.
      if (active !== dictation || dictation.isStopping()) {
        dictation.cancel();
        return { kind: 'cancelled' };
      }
    }
    // Steps 1 to 3 of capture run synchronously inside `begin`, before this
    // await; the one status read, when it is still needed, runs alongside.
    const capturePromise = dictation.begin(statusLoaded);
    const statusRead = statusLoaded ? null : deps.ensureStatus().catch(() => null);
    let capture: Capture;
    try {
      capture = await capturePromise;
    } catch (error) {
      const osStatus = isBrowserSurface()
        ? 'unknown'
        : await Promise.resolve(window.electron?.dictationMicStatus?.()).catch(() => undefined);
      const failed: DictationNotice = {
        ...micErrorNotice(error, {
          platform: window.electron?.platform ?? 'unknown',
          surface: isBrowserSurface() ? 'browser' : 'desktop',
          osStatus: osStatus ?? 'unknown',
          host: typeof location !== 'undefined' ? location.host : undefined,
        }),
        failure: true,
      };
      dictation.cancel();
      if (snapshot.key === options.key) publish({ activity: 'error', notice: failed });
      return { kind: 'failed', notice: failed };
    }
    if (statusRead) {
      await statusRead;
      const after = deps.phase();
      if (after.phase !== 'idle') {
        dictation.resolveStream(null);
        dictation.cancel();
        return outcomeFor(after);
      }
      dictation.resolveStream(capture.sampleRate);
    }
    return active === dictation ? { kind: 'recording' } : { kind: 'cancelled' };
  };
  return run();
}

/** Stop and insert (the recording pill, Enter while recording). */
export function stopDictation(key?: string): void {
  if (active && (key === undefined || active.key === key)) active.stop();
}

/** Escape while arming, recording or transcribing: nothing is inserted. */
export function cancelDictation(key?: string): void {
  if (active && (key === undefined || active.key === key)) active.cancel();
}

/** **Retry** in MSG-ENGINE-FAILED's or MSG-SERVER-BUSY-FAILED's notice row. */
export function retryDictation(key: string): Promise<void> {
  if (!kept || kept.key !== key || active) return Promise.resolve();
  const { stream, dictation, timer } = kept;
  clearTimeout(timer);
  kept = null;
  active = dictation;
  return dictation.retry(stream);
}

/**
 * A press found no speech model: the composer named here opens its setup
 * popover, never a download (DD-61). The chord asks this way, since it has no
 * popover of its own.
 */
export function requestDictationSetup(key: string): void {
  publish({ setupRequest: { key } });
}

/**
 * A press, usually the chord, that cannot start, with the reason the disabled
 * microphone's tooltip gives, so the card says why instead of nothing happening.
 * Nothing changes while any dictation runs.
 */
export function noteDictationUnavailable(key: string, reason: string): void {
  if (active) return;
  publish({
    ...IDLE,
    key,
    activity: 'error',
    notice: { id: 'server', text: reason, action: null, failure: true },
    announcement: reason,
  });
}

/** Clear the notice row, for example when the person dismisses it. */
export function dismissDictationNotice(key: string): void {
  if (snapshot.key === key && snapshot.notice && !active) {
    publish({ notice: null, activity: 'idle' });
  }
}

/** Drop a recording kept for **Retry** (the composer unmounted, or the person cancelled). */
export function dropKeptDictation(key?: string): void {
  if (kept && (key === undefined || kept.key === key)) dropKept();
}

// ── tests ───────────────────────────────────────────────────────────────────

export function resetDictationSessionForTests(overrides: Partial<Deps> = {}): void {
  sessionEra++;
  active?.cancel();
  active = null;
  running.clear();
  if (kept) clearTimeout(kept.timer);
  kept = null;
  pendingFailure = null;
  composers.clear();
  listeners.clear();
  levelListeners.clear();
  busyListeners.clear();
  level = 0;
  snapshot = IDLE;
  busyReported = false;
  lockScreenInstalled = false;
  inserting = null;
  deps = { ...DEFAULT_DEPS, ...overrides };
}

/** For tests that draw a phase without recording: the snapshot, as given. */
export function setDictationSnapshotForTests(next: Partial<DictationSnapshot>): void {
  snapshot = { ...snapshot, ...next };
  for (const listener of [...listeners]) listener();
}

/** For tests that draw the level bars: the level, as given. */
export function setDictationLevelForTests(next: number): void {
  setLevel(next);
}
