/**
 * Which windows are dictating, for the update restart (DD-65, MSG-RESTART-BUSY).
 *
 * The renderer sends `dictation:busy` while any composer in the window arms,
 * records or still waits for its text (`components/dictation/dictationSession.ts`).
 * `main.ts` records it here, and `utils/autoUpdater.ts` asks
 * {@link isDictationBusy} before **Restart & update**, so this lives in its own
 * module rather than in `main.ts`, which imports the updater.
 *
 * A window's entry must not outlive the document that set it. A new document
 * (a reload, a crash recovery) starts with nothing recording, and its preload
 * sends `false` before anything else; a renderer that crashes, or a window that
 * closes, clears the entry here, since neither will send `false` again.
 */

/** The part of `WebContents` this module needs. */
export interface BusyContents {
  readonly id: number;
  once(event: 'destroyed' | 'render-process-gone', listener: () => void): unknown;
}

const busyWindows = new Set<number>();
/** Contents whose `destroyed` and `render-process-gone` are already watched, once each. */
const watched = new WeakSet<BusyContents>();

/** Record what one window's renderer said. */
export function setDictationBusy(contents: BusyContents, busy: boolean): void {
  const id = contents.id;
  if (!busy) {
    busyWindows.delete(id);
    return;
  }
  busyWindows.add(id);
  if (watched.has(contents)) return;
  watched.add(contents);
  // Once per contents, not per dictation: a listener per `true` would pile up
  // on a window that dictates often.
  const clear = () => {
    busyWindows.delete(id);
    watched.delete(contents);
  };
  contents.once('destroyed', clear);
  contents.once('render-process-gone', clear);
}

/** Whether any window is dictating. */
export function isDictationBusy(): boolean {
  return busyWindows.size > 0;
}

/** For tests: forget every window. */
export function resetDictationBusyForTests(): void {
  busyWindows.clear();
}
