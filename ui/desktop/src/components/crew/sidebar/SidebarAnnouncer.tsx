import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from 'react';
import { useCrew } from '../state/CrewControllerContext';
import { sidebarCopy } from './copy';

/**
 * The sidebar's two live regions (T-17).
 *
 * - **Polite** (`aria-live="polite"`): changes that have no visible result where the person is
 *   looking — a "Copy …" item that closed its menu, someone starting to wait to join, a code the
 *   host entered. Copy never toasts, so its confirmation is spoken here. The region exists from
 *   the first render, empty: a live region added together with its text is not reliably heard.
 * - **Alert** (`role="alert"`): the different-code warning only. It is the one change in the
 *   sidebar a person must not miss, and it is raised once per new attempt, never on re-render.
 *   The `role="alert"` element is INSERTED with its text, which is the one live-region shape
 *   every screen reader announces on insertion; it is absent at rest, so the sidebar adds no
 *   empty alert for the rest of the page's alerts to be confused with.
 *
 *   ⚠ **Its host carries `aria-live="assertive"`, and that is load-bearing (Q2-47).** A modal
 *   (Radix, through `aria-hidden`'s `hideOthers`) hides everything outside it, and keeps only
 *   elements that carry an `aria-live` attribute when it opens. Without one, the host was hidden
 *   whenever any dialog was open, and a warning raised then — a host busy in Invite people —
 *   reached nobody. The host holds no controls, so keeping it exposed exposes nothing else.
 *
 * Each message renders in its own KEYED span, so the same sentence said twice is inserted twice
 * and heard twice (a changed text node that happens to hold the same words is not). Neither
 * region carries `role="status"`: the status row's `role="status"` is the sidebar's one status.
 */
export interface SidebarAnnounce {
  announce(message: string): void;
  alert(message: string): void;
}

const noop = () => {};
const AnnounceContext = createContext<SidebarAnnounce>({ announce: noop, alert: noop });

/** How long a message stays in its region before it is cleared for the next one. */
const ANNOUNCEMENT_MS = 2000;
const ALERT_MS = 8000;

interface Spoken {
  id: number;
  text: string;
}

/** One live message with its own clear timer; `say` replaces it and restarts the timer. */
function useLiveMessage(ttl: number): [Spoken | null, (text: string) => void] {
  const [message, setMessage] = useState<Spoken | null>(null);
  const timer = useRef<number | null>(null);
  const next = useRef(0);

  useEffect(
    () => () => {
      if (timer.current !== null) window.clearTimeout(timer.current);
    },
    []
  );

  const say = useCallback(
    (text: string) => {
      if (!text) return;
      if (timer.current !== null) window.clearTimeout(timer.current);
      next.current += 1;
      setMessage({ id: next.current, text });
      timer.current = window.setTimeout(() => {
        timer.current = null;
        setMessage(null);
      }, ttl);
    },
    [ttl]
  );
  return [message, say];
}

export function SidebarAnnouncer({ children }: { children: ReactNode }) {
  const [polite, announce] = useLiveMessage(ANNOUNCEMENT_MS);
  const [urgent, alert] = useLiveMessage(ALERT_MS);
  const value = useMemo(() => ({ announce, alert }), [announce, alert]);

  return (
    <AnnounceContext.Provider value={value}>
      {children}
      <span
        className="sr-only"
        aria-live="polite"
        aria-atomic="true"
        data-crew-sidebar-announcer=""
      >
        {polite && <span key={polite.id}>{polite.text}</span>}
      </span>
      <span className="sr-only" aria-live="assertive" data-crew-sidebar-alert="">
        {urgent && (
          <span key={urgent.id} role="alert">
            {urgent.text}
          </span>
        )}
      </span>
    </AnnounceContext.Provider>
  );
}

/** The sidebar's live regions. Outside a `SidebarAnnouncer` both calls do nothing. */
export function useSidebarAnnounce(): SidebarAnnounce {
  return useContext(AnnounceContext);
}

/**
 * Writes `text` to the clipboard. `true` when it landed; never throws. The caller decides what
 * the person sees — a copy result belongs next to what asked for it.
 */
export async function writeClipboard(text: string): Promise<boolean> {
  try {
    if (!navigator.clipboard?.writeText) return false;
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    return false;
  }
}

/**
 * Copies `text` and confirms it: "Copied" in the live region, or — when the clipboard is missing
 * or refuses — the failure in the connection bar. For a menu item that closes its menu, where no
 * control is left to show the result on.
 */
export function useSidebarCopy(): (text: string) => Promise<void> {
  const { announce } = useContext(AnnounceContext);
  const { reportError } = useCrew();
  return useCallback(
    async (text: string) => {
      if (await writeClipboard(text)) announce(sidebarCopy.clipboard.copied);
      else reportError(sidebarCopy.clipboard.failed, 'global');
    },
    [announce, reportError]
  );
}
