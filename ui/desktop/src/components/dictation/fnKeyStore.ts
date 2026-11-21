/**
 * The fn key shortcut as this window knows it (macOS): the setting and the listener's state,
 * read from main once and then kept current by main's `dictation:fn:state` broadcasts. The
 * microphone's tooltip and the Settings rows read it through `useSyncExternalStore`.
 */
import type { FnKeySettings, FnKeyView } from '../../utils/dictationFnKey';

let view: FnKeyView | null = null;
let started = false;
const listeners = new Set<() => void>();

function set(next: FnKeyView | null): void {
  if (!next) return;
  view = next;
  for (const listener of [...listeners]) listener();
}

function start(): void {
  if (started) return;
  started = true;
  const bridge = typeof window === 'undefined' ? undefined : window.electron;
  bridge?.onDictationFnState?.(set);
  void Promise.resolve(bridge?.dictationFnGet?.())
    .then((first) => {
      // A broadcast that arrived first is newer than this answer.
      if (!view) set(first ?? null);
    })
    .catch(() => undefined);
}

export function subscribeFnKey(listener: () => void): () => void {
  start();
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function getFnKeySnapshot(): FnKeyView | null {
  return view;
}

/** Write the setting through main, which answers with the new view. */
export async function setFnKeySettings(settings: FnKeySettings): Promise<void> {
  const next = await window.electron?.dictationFnSet?.(settings);
  set(next ?? null);
}

/** For tests: forget the view and the subscription. */
export function resetFnKeyStoreForTests(): void {
  view = null;
  started = false;
  listeners.clear();
}
