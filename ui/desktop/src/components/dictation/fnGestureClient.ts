/**
 * This window's side of the fn gestures (macOS): main reads fn, decides that this window is the
 * one to act (it is in front, or its dictation is the one running) and sends the intent here
 * (`utils/fnGesture.ts`, `main.ts`). This module acts on it with the composers' own controls, so
 * a gesture behaves exactly as the microphone button would: the setup popover when no model is
 * installed, the reason when dictation is unavailable, one recording at a time.
 *
 * Which composer: the one whose field has focus, else the one focused last, else the only one.
 * The microphone button stays in charge throughout: it can stop a recording fn started, and a
 * double-tap stops a recording it started.
 */
import type { FnIntent } from '../../utils/fnGesture';
import {
  activeDictationToken,
  cancelDictation,
  getDictationSnapshot,
  stopDictation,
} from './dictationSession';

export interface FnGestureTarget {
  /** The composer's own press: starts a recording, or opens setup when it cannot. */
  press: () => void;
  field: () => HTMLElement | null;
}

const targets = new Map<string, FnGestureTarget>();
let lastFocused: string | null = null;
/** The recording the current hold started, so its release stops that one and no other. */
let holdToken: object | null = null;
let focusListening = false;

function onFocusIn(event: FocusEvent): void {
  for (const [key, target] of targets) {
    if (target.field() === event.target) {
      lastFocused = key;
      return;
    }
  }
}

/** A composer offers itself to the fn gestures for as long as it is mounted. */
export function registerFnGestureTarget(key: string, target: FnGestureTarget): () => void {
  targets.set(key, target);
  if (!focusListening && typeof document !== 'undefined') {
    focusListening = true;
    document.addEventListener('focusin', onFocusIn, true);
  }
  return () => {
    if (targets.get(key) !== target) return;
    targets.delete(key);
    if (lastFocused === key) lastFocused = null;
  };
}

/** The composer a new fn dictation goes into, or null when this window shows none. */
export function fnTargetKey(): string | null {
  const focused = typeof document === 'undefined' ? null : document.activeElement;
  for (const [key, target] of targets) {
    if (focused && target.field() === focused) return key;
  }
  if (lastFocused && targets.has(lastFocused)) return lastFocused;
  if (targets.size === 1) return targets.keys().next().value ?? null;
  return null;
}

const recordingNow = (): boolean => {
  const { activity } = getDictationSnapshot();
  return activity === 'arming' || activity === 'recording';
};

function startInTarget(): boolean {
  const key = fnTargetKey();
  if (!key) return false;
  targets.get(key)!.press();
  return true;
}

export function handleFnIntent(intent: FnIntent): void {
  switch (intent) {
    case 'start-hold': {
      holdToken = null;
      if (recordingNow()) return;
      if (startInTarget()) holdToken = activeDictationToken();
      return;
    }
    case 'end-hold':
    case 'cancel-hold': {
      const token = holdToken;
      holdToken = null;
      // Only the recording this hold started: the microphone may have stopped it and started
      // another while fn was held.
      if (!token || activeDictationToken() !== token || !recordingNow()) return;
      const { key } = getDictationSnapshot();
      if (!key) return;
      if (intent === 'end-hold') stopDictation(key);
      else cancelDictation(key);
      return;
    }
    case 'toggle': {
      holdToken = null;
      const { key, activity } = getDictationSnapshot();
      if ((activity === 'arming' || activity === 'recording') && key) {
        stopDictation(key);
        return;
      }
      if (activity === 'transcribing') return;
      startInTarget();
      return;
    }
  }
}

let installed: (() => void) | null = null;

/** Listen for main's fn intents in this window. Once; answers the disposer. */
export function installFnGestureClient(): () => void {
  if (installed) return installed;
  const off = window.electron?.onDictationFnIntent?.(handleFnIntent) ?? (() => undefined);
  installed = () => {
    off();
    installed = null;
  };
  return installed;
}

/** For tests: forget every composer and hold. */
export function resetFnGestureClientForTests(): void {
  targets.clear();
  lastFocused = null;
  holdToken = null;
}
