import { useCallback, useEffect, useRef, useState } from 'react';

/**
 * How long a menu stays open showing "Copied" after a copy item, before it closes itself (Q2-34,
 * Q3-57). Long enough to be read, short enough that the menu does not linger: with the menu's exit
 * fade the whole answer takes about 700ms, the same in every sidebar menu.
 */
export const MENU_COPY_CLOSE_MS = 600;

/** How long a refused copy reads "Couldn't copy" on the item; the menu stays open meanwhile. */
export const MENU_COPY_FAILED_MS = 1500;

/** What a copy item shows: its own words, or the outcome of the copy it just made. */
export type MenuCopyState = 'idle' | 'copied' | 'failed';

export interface MenuCopyItem {
  state: MenuCopyState;
  /** The menu's `onOpenChange`: opening the menu is what resets the item to its own words. */
  onOpenChange(open: boolean): void;
  /** Runs a copy and answers on the item. Resolves to whether the copy landed. */
  run(write: () => Promise<boolean>): Promise<boolean>;
}

/**
 * A copy item that answers inside its own menu, with the message menu's timing (Q3-26, Q3-57):
 * the item reads "Copied", the menu closes {@link MENU_COPY_CLOSE_MS} later, and the label KEEPS
 * "Copied" until the menu unmounts — it is reset only when the menu opens again. The team menu
 * used to reset it in the same tick it closed, so the item read "Copy team ID" again for the
 * length of the exit fade, about 130ms before the menu was gone, as if the copy had been undone.
 * A refused copy reads "Couldn't copy" for {@link MENU_COPY_FAILED_MS} and the menu stays, so the
 * person can try again.
 *
 * `setOpen` is the menu's own open state; pass `onOpenChange` as the menu's.
 */
export function useMenuCopyItem(setOpen: (open: boolean) => void): MenuCopyItem {
  const [state, setState] = useState<MenuCopyState>('idle');
  const timer = useRef<number | null>(null);
  const clearTimer = useCallback(() => {
    if (timer.current !== null) window.clearTimeout(timer.current);
    timer.current = null;
  }, []);
  useEffect(() => clearTimer, [clearTimer]);

  const onOpenChange = useCallback(
    (open: boolean) => {
      clearTimer();
      if (open) setState('idle');
      setOpen(open);
    },
    [clearTimer, setOpen]
  );

  const run = useCallback(
    async (write: () => Promise<boolean>) => {
      const copied = await write();
      clearTimer();
      setState(copied ? 'copied' : 'failed');
      timer.current = window.setTimeout(
        () => {
          timer.current = null;
          // A landed copy closes the menu and keeps its "Copied" through the exit.
          if (copied) setOpen(false);
          else setState('idle');
        },
        copied ? MENU_COPY_CLOSE_MS : MENU_COPY_FAILED_MS
      );
      return copied;
    },
    [clearTimer, setOpen]
  );

  return { state, onOpenChange, run };
}
