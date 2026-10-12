import { useLayoutEffect } from 'react';

/**
 * What a composer states, handed to the composer that replaces it (V2-R8-3).
 *
 * ⚠ **A new chat's first send REMOUNTS the composer.** `BaseChat` draws it in
 * one of two places: centred under the greeting while the chat is empty, and in
 * the bar under the transcript once it is not (`renderChatInput()` sits in both
 * branches). The send leaves the empty state in the commit that starts creating
 * the chat, so React unmounts one `ChatInput` and mounts another, and every
 * state the first one held starts over. Measured in the running app: the Tools
 * chip read "Tools 5", then "Tools", then "Tools 5" again within 35 ms, and the
 * folder a person had picked fell back to the default for the same frames. The
 * chips' own carry from "no chat" to the chat they just started could not help:
 * it was state of the mount that was gone.
 *
 * So a composer publishes, under its lasting identity (the key that stays the
 * same from the empty chat to the started one: `BaseChat`'s tab-scoped
 * `reasoningDraftKey`, Home's draft key), what it states; and a composer that
 * mounts under that identity starts from it.
 *
 * ⚠ **A hand-over, never a cache.** An entry lives exactly as long as the
 * composer that published it: it is removed when that composer unmounts. React
 * renders the replacement BEFORE the old composer's cleanup runs, so the new
 * one reads the entry in its first render; anything mounting later (the tab
 * opened again, another visit) finds nothing and asks for itself. A value read
 * from here is only shown by a rule that already held inside one mount (the
 * same chat, or the chat this composer just started), so nothing read for one
 * chat is ever shown under another.
 */

interface Entry {
  readonly value: unknown;
}

const live = new Map<string, Entry>();

const entryId = (composerKey: string, slot: string) => `${slot}\u0000${composerKey}`;

/**
 * What the composer currently mounted under `composerKey` published for
 * `slot`, or `null`. Read it in a state initialiser: it is the value the
 * previous composer showed when this one began.
 */
export function readComposerHandOff<T>(composerKey: string | undefined, slot: string): T | null {
  if (!composerKey) return null;
  const entry = live.get(entryId(composerKey, slot));
  return entry ? (entry.value as T) : null;
}

/**
 * Publishes `value` for `slot` while this composer is mounted (`null` publishes
 * nothing). Keep `value` referentially stable (a state value), since each new
 * identity replaces the entry.
 */
export function useComposerHandOff<T>(
  composerKey: string | undefined,
  slot: string,
  value: T | null
): void {
  useLayoutEffect(() => {
    if (!composerKey || value === null) return;
    const id = entryId(composerKey, slot);
    const entry: Entry = { value };
    live.set(id, entry);
    return () => {
      // Only this composer's own entry: a replacement may already have
      // published under the same identity.
      if (live.get(id) === entry) live.delete(id);
    };
  }, [composerKey, slot, value]);
}

/** Tests only: how many entries are live (each must leave with its composer). */
export function composerHandOffSizeForTests(): number {
  return live.size;
}
