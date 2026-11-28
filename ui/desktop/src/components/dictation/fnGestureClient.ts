/**
 * This window's side of the fn gestures (macOS): main reads fn, decides that this window is the
 * one to act (it was in front at the gesture's first press, or its dictation is the one running)
 * and sends the intent here (`utils/fnGesture.ts`, `utils/dictationFnKey.ts`, `main.ts`). This
 * module acts on it with the composers' own controls, so a gesture behaves exactly as the
 * microphone button would: the setup popover when no model is installed, the reason when
 * dictation is unavailable, one recording at a time.
 *
 * Which field (`fieldTarget.ts`): the focused one, the composer's or any other editable field
 * (never a password); else the active pane's composer; else the composer focused last while its
 * field is live; else the only live one; inside an open modal dialog, only its fields. A field
 * that no composer owns (a rename box, a search field) gets a session composer of its own for as
 * long as a dictation owes it text, with public terms only (`target_kind: external`), and what
 * that dictation ends with shows in the visible composer.
 *
 * The microphone button stays in charge throughout: it can stop a recording fn started, and a
 * double-tap stops a recording it started.
 *
 * At the first fn press in a window, once per profile, the composer shows MSG-FN-CONFLICT (or
 * MSG-FN-CONFLICT-UNKNOWN) when macOS also acts on fn, because a double-tap runs that action
 * twice ({@link getFnConflictNote}), but only while fn alone dictates: macOS's own action is a
 * bare fn's business (DD-101).
 *
 * When a bare fn hold was left to another app that also uses fn (main's `use-alternative`
 * notice, DD-101), the same slot shows once per profile which key dictates in Biorouter:
 * MSG-FN-USE-ALT-COLLISION for each app Biorouter's own list names, MSG-FN-USE-ALT-CHOSEN when the
 * person chose the modifier ({@link handleFnNotice}).
 */
import { setFieldTextByState, type DictationField } from '../../utils/dictatedText';
import { fnKeyLabel } from '../../utils/dictationFnKey';
import type { FnIntent } from '../../utils/fnGesture';
import { dictationMessage, FN_USAGE_ACTIONS } from './copy';
import {
  activeDictationToken,
  attachDictationComposer,
  canStartDictationQuietly,
  cancelDictation,
  getDictationSnapshot,
  isDictationHolding,
  noteDictationUnavailable,
  requestDictationSetup,
  startDictation,
  stopDictation,
  subscribeDictation,
} from './dictationSession';
import { noteDictationInterest } from './dictationStore';
import {
  chooseFnComposer,
  focusedLooseField,
  isLiveField,
  visibleComposer,
  type FieldCandidate,
} from './fieldTarget';
import { getFnKeySnapshot, subscribeFnKey } from './fnKeyStore';

export interface FnGestureTarget {
  /** The composer's own press: starts a recording, or opens setup when it cannot. */
  press: () => void;
  field: () => DictationField | null;
  /** The composer draws the one-time fn note itself (the chat composer and Home do). */
  showsNotes?: boolean;
}

/** The localStorage key that says the fn note was shown on this profile (DD-98). */
export const FN_CONFLICT_SHOWN_KEY = 'biorouter:fn-conflict-shown';

/**
 * The localStorage key that lists the key notices shown on this profile (DD-101): `chosen`, and
 * `collision:<bundle id>` for each app of Biorouter's own list.
 */
export const FN_ALT_NOTICE_SHOWN_KEY = 'biorouter:fn-alt-notice-shown';

/** The one-time fn note: the composer that shows it, and its sentence. */
export interface FnConflictNote {
  key: string;
  text: string;
}

const targets = new Map<string, FnGestureTarget>();
let lastFocused: string | null = null;
/**
 * The recording the current press started (at fn down, or at the hold threshold), so its release,
 * a tap or another key acts on that one and no other.
 */
let holdToken: object | null = null;
/**
 * The capture a tap's release asked to discard. A double-tap's toggle that still finds it running
 * (it is closing) cancels it and starts a fresh recording, rather than stopping it as if it were a
 * dictation the person meant to finish.
 */
let discardedToken: object | null = null;
/**
 * A start that came while this window's last dictation still transcribed (F14, DD-102): a
 * confirmed hold, or a double-tap's hands-free start. One waits at most, and it starts the moment
 * that dictation's text is in, if the gesture still wants it: a release or another key drops a
 * held one, a second double-tap drops a hands-free one, and a failure (whose notice offers Retry)
 * drops either.
 */
let waitingStart: 'hold' | 'hands-free' | null = null;
let waitingWatch: (() => void) | null = null;
let focusListening = false;

/** Fields no composer owns, each with the session composer fn dictates into it through. */
const loose = new Map<string, { element: DictationField; detach: () => void }>();
let looseSeq = 0;
let looseWatch: (() => void) | null = null;

let conflictNote: FnConflictNote | null = null;
/** This window decided about the fn note: it showed it, or there was nothing to show. */
let conflictDecided = false;
/** The key notices this window showed, for a profile whose storage cannot be read. */
const altNoticesShown = new Set<string>();
const noteListeners = new Set<() => void>();

function candidates(): FieldCandidate[] {
  return [...targets].map(([key, target]) => ({ key, field: target.field }));
}

/** The composer whose field is `node` or holds it, or null. */
function ownerOf(node: EventTarget | null): string | null {
  if (!node || typeof (node as Node).nodeType !== 'number') return null;
  for (const [key, target] of targets) {
    const field = target.field();
    if (field && (field === node || field.contains(node as Node))) return key;
  }
  return null;
}

function onFocusIn(event: FocusEvent): void {
  const key = ownerOf(event.target);
  if (key) lastFocused = key;
}

/**
 * Focus moved from a composer to an element that is no composer's field (the Edit button of a
 * message editor that closed, a toolbar button, another field): the person works elsewhere now.
 * A window that loses focus keeps its field focused and names no next element, so that keeps it.
 */
function onFocusOut(event: FocusEvent): void {
  const next = event.relatedTarget;
  if (next && typeof (next as Node).nodeType === 'number' && ownerOf(next) === null) {
    lastFocused = null;
  }
}

/** A composer offers itself to the fn gestures while its field exists. */
export function registerFnGestureTarget(key: string, target: FnGestureTarget): () => void {
  targets.set(key, target);
  if (!focusListening && typeof document !== 'undefined') {
    focusListening = true;
    document.addEventListener('focusin', onFocusIn, true);
    document.addEventListener('focusout', onFocusOut, true);
  }
  return () => {
    if (targets.get(key) !== target) return;
    targets.delete(key);
    if (lastFocused === key) lastFocused = null;
  };
}

/** Let go of loose fields that no dictation owes text any more (but `keep`, about to be used). */
function sweepLoose(keep?: DictationField | null): void {
  for (const [key, entry] of loose) {
    if (entry.element === keep || isDictationHolding(key)) continue;
    loose.delete(key);
    entry.detach();
  }
  if (loose.size === 0 && looseWatch) {
    looseWatch();
    looseWatch = null;
  }
}

/** The session composer for a field no composer owns, made the first time fn chooses it. */
function looseKey(element: DictationField): string {
  for (const [key, entry] of loose) if (entry.element === element) return key;
  const key = `field:${++looseSeq}`;
  const control =
    element.localName === 'textarea' || element.localName === 'input'
      ? (element as HTMLTextAreaElement | HTMLInputElement)
      : null;
  const detach = attachDictationComposer({
    key,
    // Only while the text can still go there: a box that closed, or turned read only, has none.
    field: () => (isLiveField(element) ? element : null),
    // No React state to write: the field's own value setter, read back.
    setByState: (value, caret) => (control ? setFieldTextByState(control, value, caret) : false),
    storesDraft: false,
    loose: true,
  });
  loose.set(key, { element, detach });
  looseWatch ??= subscribeDictation(() => sweepLoose());
  return key;
}

/** The key a new fn dictation goes into, or null when this window shows no field for it. */
export function fnTargetKey(): string | null {
  if (typeof document === 'undefined') return null;
  const fields = candidates();
  if (lastFocused && !isLiveField(targets.get(lastFocused)?.field() ?? null)) lastFocused = null;
  const field = focusedLooseField(fields);
  sweepLoose(field);
  if (field) return looseKey(field);
  return chooseFnComposer({ candidates: fields, lastFocused });
}

const recordingNow = (): boolean => {
  const { activity } = getDictationSnapshot();
  return activity === 'arming' || activity === 'recording';
};

/** The visible composer, where a loose field's setup popover or reason shows. */
const visibleHome = (): string | null => visibleComposer(candidates());

/** A loose field's press: what a composer's microphone does, with public terms only. */
function pressLoose(key: string): void {
  const { key: current, activity } = getDictationSnapshot();
  if (current === key && activity === 'transcribing') return;
  const holding = isDictationHolding(key);
  void startDictation({ key, target: { target_kind: 'external' } }).then((outcome) => {
    const home = visibleHome();
    if (!home) return;
    if (outcome.kind === 'setup') requestDictationSetup(home);
    else if (outcome.kind === 'unavailable' && outcome.reason) {
      noteDictationUnavailable(home, outcome.reason);
    }
  });
  // After the capture has started, as the composers' own press does (`useDictationComposer.ts`).
  if (!holding) noteDictationInterest();
}

function startInTarget(): boolean {
  const key = fnTargetKey();
  if (!key) return false;
  const target = targets.get(key);
  if (target) target.press();
  else pressLoose(key);
  return true;
}

/** The recording this press started is still the one running. */
const holdIsLive = (token: object | null): boolean =>
  token !== null && activeDictationToken() === token && recordingNow();

const transcribingNow = (): boolean => getDictationSnapshot().activity === 'transcribing';

function waitForTranscript(kind: 'hold' | 'hands-free'): void {
  waitingStart = kind;
  waitingWatch ??= subscribeDictation(onSessionChange);
}

function dropWaitingStart(): void {
  waitingStart = null;
  waitingWatch?.();
  waitingWatch = null;
}

/** The session moved on: the start that waited begins once the text is in, and only then. */
function onSessionChange(): void {
  if (waitingStart === null) return;
  const { activity } = getDictationSnapshot();
  if (activity === 'transcribing') return;
  const kind = waitingStart;
  dropWaitingStart();
  if (activity !== 'idle') return;
  // After the publish that brought the session to rest, not inside it.
  queueMicrotask(() => {
    if (recordingNow() || transcribingNow()) return;
    if (startInTarget() && kind === 'hold') holdToken = activeDictationToken();
  });
}

export function handleFnIntent(intent: FnIntent): void {
  switch (intent) {
    case 'prime': {
      // fn went down: start capturing now, so the first words of a hold are kept, but only when
      // that opens nothing (no setup popover, notice or prompt) for a press that may be a tap.
      holdToken = null;
      const started = !recordingNow() && canStartDictationQuietly() && startInTarget();
      if (started) holdToken = activeDictationToken();
      noteFnConflictOnce(started ? getDictationSnapshot().key : null);
      return;
    }
    case 'start-hold': {
      // The hold is confirmed. A capture its press started goes on; otherwise it starts now, as
      // the microphone button would, setup popover and all.
      discardedToken = null;
      if (holdIsLive(holdToken)) return;
      holdToken = null;
      if (recordingNow()) return;
      // This window's last dictation still transcribes: a press now would only stop it again, so
      // the start waits for its text (F14). A start already waiting keeps its place.
      if (transcribingNow()) {
        if (waitingStart === null) waitForTranscript('hold');
        return;
      }
      if (startInTarget()) holdToken = activeDictationToken();
      return;
    }
    case 'end-hold':
    case 'cancel-hold': {
      // A release or another key before the waiting hold began: nothing was recorded, so it goes.
      if (waitingStart === 'hold') dropWaitingStart();
      const token = holdToken;
      holdToken = null;
      if (intent === 'end-hold') discardedToken = null;
      // Only the recording this press started: the microphone may have stopped it and started
      // another while fn was held. A tap's release and fn with another key discard it.
      if (!holdIsLive(token)) return;
      const { key } = getDictationSnapshot();
      if (!key) return;
      if (intent === 'end-hold') {
        stopDictation(key);
      } else {
        discardedToken = token;
        cancelDictation(key);
      }
      return;
    }
    case 'toggle': {
      const token = holdToken;
      const discarded = discardedToken;
      holdToken = null;
      discardedToken = null;
      // The double-tap's second press already started a recording: it goes on, hands-free.
      if (holdIsLive(token)) return;
      const { key, activity } = getDictationSnapshot();
      if ((activity === 'arming' || activity === 'recording') && key) {
        if (discarded !== null && activeDictationToken() === discarded) {
          // What runs is the first tap's capture, still closing: no dictation the person meant.
          // It goes, and this double-tap starts the one it asks for.
          cancelDictation(key);
          if (!recordingNow()) startInTarget();
          return;
        }
        stopDictation(key);
        return;
      }
      if (activity === 'transcribing') {
        // The start waits for this window's text (F14); a second double-tap stops what waits.
        if (waitingStart === 'hands-free') dropWaitingStart();
        else waitForTranscript('hands-free');
        return;
      }
      startInTarget();
      return;
    }
  }
}

// ── the one-time fn note (MSG-FN-CONFLICT) ──────────────────────────────────

function readConflictShown(): boolean {
  try {
    return window.localStorage.getItem(FN_CONFLICT_SHOWN_KEY) !== null;
  } catch {
    // No storage: decided per window, so the note shows at most once in this one.
    return false;
  }
}

function writeConflictShown(): void {
  try {
    window.localStorage.setItem(FN_CONFLICT_SHOWN_KEY, '1');
  } catch {
    // No storage: this window still remembers it showed the note.
  }
}

/** The composer that shows the note: the gesture's own when it draws notes, else the visible one that does. */
function noteHome(targetKey: string | null): string | null {
  if (targetKey && targets.get(targetKey)?.showsNotes) return targetKey;
  return visibleComposer(candidates().filter(({ key }) => targets.get(key)?.showsNotes === true));
}

/**
 * At the first fn press in this window: when macOS also acts on fn (`AppleFnUsageType` unset, or
 * set to change the input source, show emoji or start its own dictation) and this profile has not
 * seen the note, the composer shows it. Decided once per window; a press before the fn status is
 * known, or with no composer to show it in, leaves it to the next press.
 */
function noteFnConflictOnce(targetKey: string | null): void {
  if (conflictDecided) return;
  const view = getFnKeySnapshot();
  if (!view) return;
  // Only the modifier with fn dictates: macOS's fn action is not this press's, so the note waits
  // for a press of fn alone. A view from before main said which key counts as fn alone.
  if (view.active && !view.active.bare) return;
  const usage = view.status.fnUsage;
  if (usage === 0 || readConflictShown()) {
    conflictDecided = true;
    return;
  }
  const home = noteHome(targetKey);
  if (!home) return;
  conflictDecided = true;
  writeConflictShown();
  conflictNote = {
    key: home,
    text:
      usage === null
        ? dictationMessage('MSG-FN-CONFLICT-UNKNOWN')
        : dictationMessage('MSG-FN-CONFLICT', { action: FN_USAGE_ACTIONS[usage] }),
  };
  for (const listener of [...noteListeners]) listener();
}

// ── the one-time key notice (MSG-FN-USE-ALT-COLLISION, MSG-FN-USE-ALT-CHOSEN) ─────

function readAltNoticesShown(): string[] {
  try {
    const raw = window.localStorage.getItem(FN_ALT_NOTICE_SHOWN_KEY);
    if (raw === null) return [];
    const parsed: unknown = JSON.parse(raw);
    return Array.isArray(parsed)
      ? parsed.filter((entry): entry is string => typeof entry === 'string').slice(0, 64)
      : [];
  } catch {
    return [];
  }
}

function writeAltNoticeShown(id: string): void {
  const shown = readAltNoticesShown();
  if (!shown.includes(id)) shown.push(id);
  const value = JSON.stringify(shown);
  try {
    window.localStorage.setItem(FN_ALT_NOTICE_SHOWN_KEY, value);
  } catch {
    // No storage: this window still remembers it showed the notice.
  }
}

/**
 * Main's `use-alternative`: a bare fn hold in this window was left to another app that also uses
 * fn, and the press started nothing. Once per profile per reason and app, the composer the press
 * was for says which key dictates in Biorouter. Nothing shows for the fn choice, while fn alone
 * dictates, or before main has said which key dictates.
 */
export function handleFnNotice(notice: 'use-alternative'): void {
  if (notice !== 'use-alternative') return;
  const view = getFnKeySnapshot();
  const active = view?.active;
  if (!view || !active || active.bare || active.modifier === null) return;
  if (view.settings.combination === 'fn') return;
  const alt = fnKeyLabel(active.modifier);
  let id: string;
  let text: string;
  if (active.reason === 'collision' && active.collidingApp) {
    id = `collision:${active.collidingApp.bundle}`;
    text = dictationMessage('MSG-FN-USE-ALT-COLLISION', { app: active.collidingApp.name, alt });
  } else if (active.reason === 'chosen') {
    id = 'chosen';
    text = dictationMessage('MSG-FN-USE-ALT-CHOSEN', { alt });
  } else {
    return;
  }
  if (altNoticesShown.has(id) || readAltNoticesShown().includes(id)) return;
  // The composer the press would have gone to, without making a loose field its own composer.
  const home = noteHome(chooseFnComposer({ candidates: candidates(), lastFocused }));
  if (!home) return;
  altNoticesShown.add(id);
  writeAltNoticeShown(id);
  conflictNote = { key: home, text };
  for (const listener of [...noteListeners]) listener();
}

/** The fn note this window shows, or null. A stable object: read it with {@link subscribeFnConflictNote}. */
export function getFnConflictNote(): FnConflictNote | null {
  return conflictNote;
}

export function subscribeFnConflictNote(listener: () => void): () => void {
  noteListeners.add(listener);
  return () => noteListeners.delete(listener);
}

/** **Dismiss**, or the person followed the note's link. */
export function dismissFnConflictNote(): void {
  if (!conflictNote) return;
  conflictNote = null;
  for (const listener of [...noteListeners]) listener();
}

let installed: (() => void) | null = null;

/** Listen for main's fn intents in this window. Once; answers the disposer. */
export function installFnGestureClient(): () => void {
  if (installed) return installed;
  const off = window.electron?.onDictationFnIntent?.(handleFnIntent) ?? (() => undefined);
  const offNotice = window.electron?.onDictationFnNotice?.(handleFnNotice) ?? (() => undefined);
  // The fn status (`fnUsage`) the first press's note reads, loaded before that press.
  const offStatus = subscribeFnKey(() => undefined);
  installed = () => {
    off();
    offNotice();
    offStatus();
    installed = null;
  };
  return installed;
}

/** For tests: forget every composer, field, hold and note. */
export function resetFnGestureClientForTests(): void {
  targets.clear();
  for (const entry of loose.values()) entry.detach();
  loose.clear();
  looseWatch?.();
  looseWatch = null;
  lastFocused = null;
  holdToken = null;
  discardedToken = null;
  dropWaitingStart();
  conflictNote = null;
  conflictDecided = false;
  altNoticesShown.clear();
  noteListeners.clear();
}
