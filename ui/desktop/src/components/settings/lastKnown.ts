import { useCallback, useState } from 'react';

/**
 * The settings whose value lives somewhere else (the daemon's config, the main process) and so
 * arrives a moment after a tab mounts. One key each; the value type is the caller's.
 */
export type LastKnownKey =
  | 'approvalMode'
  | 'maxTurns'
  | 'announceOnly'
  | 'spellcheck'
  | 'menuBarIcon'
  | 'dockIcon'
  | 'wakelock';

/**
 * The last answer each setting got, for the life of this window. A value is `undefined` only
 * until something has read it; `null` is a real answer ("nothing saved") and is kept as one.
 */
const known = new Map<LastKnownKey, unknown>();

/** What this window last heard for `key`, or `undefined` if nothing has read it yet. */
export function recall<T>(key: LastKnownKey): T | undefined {
  return known.get(key) as T | undefined;
}

/** Record an answer: the one a read just returned, or the one a person just saved. */
export function remember<T>(key: LastKnownKey, value: T): void {
  if (value !== undefined) known.set(key, value);
}

/**
 * Record an answer only if nothing has one yet. For a read nobody is waiting on (the warm-up in
 * `warmSettings.ts`): a control's own read, or a switch the person has since flipped, is newer
 * than it and must win.
 */
export function rememberIfUnknown<T>(key: LastKnownKey, value: T): void {
  if (!known.has(key)) remember(key, value);
}

/** Test-only: forget everything, as a fresh window would. Called from `src/test/setup.ts`. */
export function resetLastKnownForTests(): void {
  known.clear();
}

/**
 * State for a setting that is read after the page mounts, seeded with the last answer this window
 * heard (vision round 7 and 8, V4-R7-2).
 *
 * Settings mounts only the tab on screen, so every visit to Chat or App is a new mount, and a
 * control that starts from "not read yet" is held blank (a select with no words, a field with no
 * number, a switch that is not painted) for the one or two frames before the answer comes back,
 * every time. The answer cannot have changed since the last visit, so the control starts from
 * it. The read still runs on every mount and replaces the seed: this is a head start, not a
 * cache a control may trust, and a value changed elsewhere (the CLI, another window) is shown
 * correctly one answer later.
 *
 * The first read in a window still has nothing to start from, which is why `warmSettings.ts`
 * asks for these while Settings opens. Never a guess: a placeholder `true` would paint the coral
 * ON track for a person who turned the setting off (V4-01), so an unread setting stays
 * `undefined` and the control holds its place.
 *
 * The setter writes through, so a change the person makes is what the next visit starts from.
 */
export function useLastKnown<T>(key: LastKnownKey): readonly [T | undefined, (value: T) => void] {
  const [value, setValue] = useState<T | undefined>(() => recall<T>(key));
  const set = useCallback(
    (next: T) => {
      remember(key, next);
      setValue(next);
    },
    [key]
  );
  return [value, set] as const;
}
