import { useCallback, useEffect, useRef, useState, type ReactNode } from 'react';
import { filesCopy } from './copy';

/** How long "Copied" stays in the live region, matching `CopyField`. */
const ANNOUNCE_MS = 2000;

/**
 * Copy from a menu item ("Copy file ID", "Copy SHA-256") and confirm it without a toast.
 *
 * The menu closes on select, so the confirmation cannot sit on the control the way
 * `CopyField`'s does; it goes to a polite live region the caller renders once. On a clipboard
 * failure the region says "Copy failed" — never a silent no-op.
 */
export function useCopyAnnouncer(): { copy(text: string): Promise<void>; region: ReactNode } {
  const [announcement, setAnnouncement] = useState('');
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  useEffect(
    () => () => {
      if (timer.current) clearTimeout(timer.current);
    },
    []
  );
  const copy = useCallback(async (text: string) => {
    let outcome: string = filesCopy.copied;
    try {
      if (!navigator.clipboard?.writeText) throw new Error('clipboard unavailable');
      await navigator.clipboard.writeText(text);
    } catch {
      outcome = filesCopy.copyFailed;
    }
    if (timer.current) clearTimeout(timer.current);
    setAnnouncement(outcome);
    timer.current = setTimeout(() => {
      timer.current = null;
      setAnnouncement('');
    }, ANNOUNCE_MS);
  }, []);
  const region = (
    <span className="sr-only" aria-live="polite" aria-atomic="true">
      {announcement}
    </span>
  );
  return { copy, region };
}
