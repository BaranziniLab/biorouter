import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from 'react';
import { useCrew } from '../state/CrewControllerContext';
import { sidebarCopy } from './copy';

/**
 * One polite live region for the sidebar's menu actions that have no visible result: a "Copy …"
 * item closes its menu, so its confirmation is spoken rather than toasted (copy never toasts).
 * A clipboard refusal is an error the person must see, so it goes to the controller's error slot
 * (the connection bar) instead.
 */
const AnnounceContext = createContext<(message: string) => void>(() => {});

/** How long an announcement stays in the region before it is cleared for the next one. */
const ANNOUNCEMENT_MS = 2000;

export function SidebarAnnouncer({ children }: { children: ReactNode }) {
  const [message, setMessage] = useState('');
  const timer = useRef<number | null>(null);

  useEffect(
    () => () => {
      if (timer.current !== null) window.clearTimeout(timer.current);
    },
    []
  );

  const announce = useCallback((next: string) => {
    if (timer.current !== null) window.clearTimeout(timer.current);
    setMessage(next);
    timer.current = window.setTimeout(() => {
      timer.current = null;
      setMessage('');
    }, ANNOUNCEMENT_MS);
  }, []);

  return (
    <AnnounceContext.Provider value={announce}>
      {children}
      <span className="sr-only" aria-live="polite" data-crew-sidebar-announcer="">
        {message}
      </span>
    </AnnounceContext.Provider>
  );
}

/**
 * Copies `text` and confirms it: "Copied" in the live region, or — when the clipboard is missing
 * or refuses — the failure in the connection bar.
 */
export function useSidebarCopy(): (text: string) => Promise<void> {
  const announce = useContext(AnnounceContext);
  const { reportError } = useCrew();
  return useCallback(
    async (text: string) => {
      try {
        if (!navigator.clipboard?.writeText) throw new Error('clipboard unavailable');
        await navigator.clipboard.writeText(text);
        announce(sidebarCopy.clipboard.copied);
      } catch {
        reportError(sidebarCopy.clipboard.failed, 'global');
      }
    },
    [announce, reportError]
  );
}
