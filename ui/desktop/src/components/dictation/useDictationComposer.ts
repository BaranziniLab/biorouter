/**
 * What a composer needs to dictate (DD-60 to DD-63): it attaches itself to the
 * session (`dictationSession.ts`), and gets back its gate (whether dictation
 * holds the composer) and the handlers its field and microphone call.
 *
 * The composer re-renders only when its gate changes: when a dictation starts
 * and when it ends, and around a recording into an empty field, whose
 * placeholder says "Listening…". Never on the stop itself otherwise, the timer,
 * the partial line, a notice or the level: those belong to the microphone and
 * the notice row, which
 * subscribe on their own. A chat composer is a large component, and a render
 * per second of speech would be input lag for a person typing while dictating.
 *
 * Every press starts or stops inside the gesture (`pointerdown` on the
 * microphone, `keydown` for the chord), so the person's press to release time
 * is off both paths, and `getUserMedia` (and `AudioContext.resume()` where it
 * needs one, `capture.ts`) still run inside a user activation. A press notes
 * interest in dictation only after the capture has started.
 */
import { useCallback, useEffect, useMemo, useRef, useSyncExternalStore } from 'react';
import type { KeyboardEvent as ReactKeyboardEvent } from 'react';
import { isDictationChord } from '../../utils/keyboardShortcuts';
import { isBrowserSurface } from '../../utils/surface';
import {
  attachDictationComposer,
  cancelDictation,
  dictationActivityFor,
  noteDictationUnavailable,
  requestDictationSetup,
  startDictation,
  stopDictation,
  subscribeDictation,
  type DictationActivity,
  type DictationInserted,
  type DictationTarget,
} from './dictationSession';
import { noteDictationInterest } from './dictationStore';

/** A hold of the chord at least this long talks while held (DD-62); a shorter one toggles. */
export const DICTATION_HOLD_MS = 500;

/** `context` is at most this many bytes of UTF-8 (`POST /dictation/streams`). */
export const DICTATION_CONTEXT_BYTES = 2048;

/**
 * The end of `text`, at most `maxBytes` of UTF-8, cut on a character boundary:
 * the words nearest the anchor are the ones that help ranking.
 */
export function dictationContext(text: string, maxBytes = DICTATION_CONTEXT_BYTES): string {
  if (!text) return '';
  const encoder = new TextEncoder();
  if (encoder.encode(text).length <= maxBytes) return text;
  let bytes = 0;
  let start = text.length;
  for (const char of Array.from(text).reverse()) {
    const size = encoder.encode(char).length;
    if (bytes + size > maxBytes) break;
    bytes += size;
    start -= char.length;
  }
  return text.slice(start);
}

export interface DictationComposerOptions {
  /** The session key: the composer's draft key, or another key unique in this window. */
  key: string;
  textAreaRef: React.RefObject<HTMLTextAreaElement | null>;
  /** Whether `key` is a stored draft that receives the text after the composer unmounts. */
  storesDraft: boolean;
  /** What the stream is for, read at the press (DD-30 is the daemon's; this only names it). */
  target: () => DictationTarget;
  /** The composer's state path (DD-63 step 4): the field's new text and the caret. */
  setByState: (value: string, caret: number) => void;
  isComposing?: () => boolean;
  /** Characters the stored draft holds before the field's text (a `/bug ` chip). */
  draftPrefix?: () => number;
  onInserted?: (event: DictationInserted) => void;
  /**
   * Whether the field is empty, for the "Listening…" placeholder. Given, the composer also
   * renders when a recording starts or stops into an empty field; otherwise only when its
   * gates open or close.
   */
  isEmpty?: () => boolean;
}

/**
 * What a composer's render depends on: `free`, `holding` (arming, recording or transcribing:
 * the submit gates hold) or `listening` (recording into an empty field).
 */
export type DictationGate = 'free' | 'holding' | 'listening';

export interface DictationComposerControl {
  key: string;
  /** False on a `serve` browser until PR-21 lands its gates (DD-94): nothing renders. */
  available: boolean;
  /** This composer's gate; the composer renders only when it changes. */
  gate: DictationGate;
  /** Arming, recording or transcribing: the submit gates hold. */
  holdsComposer: boolean;
  /** Recording into an empty field: the placeholder says "Listening…". */
  listening: boolean;
  /** The microphone's press (`pointerdown`, or a click from the keyboard). */
  press: () => void;
  stop: () => void;
  cancel: () => void;
  /**
   * The field's own `onKeyDown`, first: the chord, Enter and Escape while
   * dictation holds the composer. True when it took the key.
   */
  handleKeyDown: (event: ReactKeyboardEvent<HTMLElement>) => boolean;
  /** A sign that dictation may be used: loads the status once and warms the device list. */
  noteInterest: () => void;
}

const holding = (activity: DictationActivity): boolean =>
  activity === 'arming' || activity === 'recording' || activity === 'transcribing';

export function useDictationComposer(options: DictationComposerOptions): DictationComposerControl {
  const { key, textAreaRef, storesDraft } = options;
  const available = !isBrowserSurface();
  const latest = useRef(options);
  latest.current = options;

  // Reduced to the gate, so the stop (recording to transcribing) renders nothing here unless
  // the placeholder must change: the stop's `finish` is already on the wire by then, and the
  // microphone draws Transcribing on its own.
  const gate = useSyncExternalStore(subscribeDictation, (): DictationGate => {
    const activity = dictationActivityFor(key);
    if (!holding(activity)) return 'free';
    return activity === 'recording' && latest.current.isEmpty?.() === true
      ? 'listening'
      : 'holding';
  });

  useEffect(() => {
    if (!available) return;
    return attachDictationComposer({
      key,
      field: () => textAreaRef.current,
      setByState: (value, caret) => latest.current.setByState(value, caret),
      isComposing: () => latest.current.isComposing?.() ?? false,
      storesDraft,
      draftPrefix: () => latest.current.draftPrefix?.() ?? 0,
      onInserted: (event) => latest.current.onInserted?.(event),
    });
  }, [available, key, storesDraft, textAreaRef]);

  const interested = useRef(false);
  const noteInterest = useCallback(() => {
    if (!available) return;
    interested.current = true;
    noteDictationInterest();
  }, [available]);

  const press = useCallback(() => {
    if (!available) return;
    const current = dictationActivityFor(key);
    if (current === 'transcribing') return;
    void startDictation({ key, target: latest.current.target() }).then((outcome) => {
      if (outcome.kind === 'setup') requestDictationSetup(key);
      else if (outcome.kind === 'unavailable' && outcome.reason) {
        noteDictationUnavailable(key, outcome.reason);
      }
    });
    // After the capture has started, never before it: interest noted first would make the
    // capture context inside the press, beside `getUserMedia`, which holds the device back
    // (`capture.ts`). A press that stops needs none.
    if (!holding(current)) noteDictationInterest();
  }, [available, key]);

  const stop = useCallback(() => stopDictation(key), [key]);
  const cancel = useCallback(() => cancelDictation(key), [key]);

  // The chord's hold: a key up of D, Shift or the platform modifier at 500 ms or
  // more stops and inserts; a shorter one leaves the recording on (a tap). A
  // window blur or a hidden page during the hold stops and inserts, because no
  // key up will come.
  const endHold = useRef<(() => void) | null>(null);
  useEffect(() => () => endHold.current?.(), []);

  const watchHold = useCallback(() => {
    endHold.current?.();
    const started = performance.now();
    const finish = (stopNow: boolean) => {
      window.removeEventListener('keyup', onKeyUp, true);
      window.removeEventListener('blur', onAway);
      document.removeEventListener('visibilitychange', onHidden);
      endHold.current = null;
      if (stopNow) stopDictation(key);
    };
    const onKeyUp = (event: KeyboardEvent) => {
      if (
        event.code === 'KeyD' ||
        event.key === 'Shift' ||
        event.key === 'Meta' ||
        event.key === 'Control'
      ) {
        finish(performance.now() - started >= DICTATION_HOLD_MS);
      }
    };
    const onAway = () => finish(true);
    const onHidden = () => {
      if (document.visibilityState === 'hidden') finish(true);
    };
    window.addEventListener('keyup', onKeyUp, true);
    window.addEventListener('blur', onAway);
    document.addEventListener('visibilitychange', onHidden);
    endHold.current = () => finish(false);
  }, [key]);

  const handleKeyDown = useCallback(
    (event: ReactKeyboardEvent<HTMLElement>): boolean => {
      if (!available) return false;
      const native = event.nativeEvent as KeyboardEvent;
      const composing = native.isComposing || latest.current.isComposing?.() === true;
      const current = dictationActivityFor(key);
      const chord = isDictationChord({
        code: native.code,
        metaKey: native.metaKey,
        ctrlKey: native.ctrlKey,
        shiftKey: native.shiftKey,
        altKey: native.altKey,
        repeat: native.repeat,
        isComposing: composing,
      });
      if (chord) {
        event.preventDefault();
        const wasHolding = holding(current);
        // `press` notes the interest itself, once the capture has started.
        interested.current = true;
        press();
        if (!wasHolding && dictationActivityFor(key) === 'arming') watchHold();
        return true;
      }
      if (!interested.current) noteInterest();
      if (composing || !holding(current)) return false;
      if (event.key === 'Enter') {
        // Enter while recording stops and inserts; it never sends (DD-62).
        event.preventDefault();
        if (current === 'recording') stopDictation(key);
        return true;
      }
      if (event.key === 'Escape') {
        // Cancels, and the sidebar overlay's Escape handler skips a handled key.
        event.preventDefault();
        cancelDictation(key);
        return true;
      }
      return false;
    },
    [available, key, noteInterest, press, watchHold]
  );

  return useMemo(
    () => ({
      key,
      available,
      gate,
      holdsComposer: gate !== 'free',
      listening: gate === 'listening',
      press,
      stop,
      cancel,
      handleKeyDown,
      noteInterest,
    }),
    [key, available, gate, press, stop, cancel, handleKeyDown, noteInterest]
  );
}
