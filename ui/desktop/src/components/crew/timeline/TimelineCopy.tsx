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
import { Button } from '../../ui/button';
import { Tooltip, TooltipContent, TooltipTrigger } from '../../ui/Tooltip';
import { Check, Copy, X } from '../../icons/app-icons';
import { cn } from '../../../utils';
import { timelineCopy } from './copy';

/**
 * Copying from the timeline — a message's text, a code block, a message or task
 * ID, a task's error — confirms itself without a toast: the control that copied
 * says so itself for two seconds ("Copied" with a check, or "Couldn't copy"),
 * and ONE polite region per timeline says "Copied" (or how to copy by hand when
 * the clipboard refuses). The region sits outside the log, so an announcement
 * never reads as a new message. A refused copy is never silent.
 */

/** How long a copy control shows its check, as `CopyField` does. */
export const COPY_FEEDBACK_MS = 2000;

async function writeClipboard(text: string): Promise<void> {
  // Absent in an insecure context, and it rejects without focus or permission:
  // a check AND a catch.
  if (!navigator.clipboard?.writeText) throw new Error('clipboard unavailable');
  await navigator.clipboard.writeText(text);
}

interface TimelineCopyApi {
  /** Copy `text`, announce the outcome, and resolve whether it landed. */
  copy(text: string): Promise<boolean>;
}

const TimelineCopyContext = createContext<TimelineCopyApi | null>(null);

export function TimelineCopyProvider({ children }: { children: ReactNode }) {
  const [announcement, setAnnouncement] = useState<{ text: string; count: number }>({
    text: '',
    count: 0,
  });
  const clearTimer = useRef<number | null>(null);
  useEffect(
    () => () => {
      if (clearTimer.current !== null) window.clearTimeout(clearTimer.current);
    },
    []
  );

  const copy = useCallback(async (text: string) => {
    let copied = true;
    try {
      await writeClipboard(text);
    } catch {
      copied = false;
    }
    // A new count re-announces the same words on a second copy.
    setAnnouncement((previous) => ({
      text: copied ? timelineCopy.copied : timelineCopy.copyFailed,
      count: previous.count + 1,
    }));
    if (clearTimer.current !== null) window.clearTimeout(clearTimer.current);
    clearTimer.current = window.setTimeout(
      () => setAnnouncement((previous) => ({ ...previous, text: '' })),
      COPY_FEEDBACK_MS
    );
    return copied;
  }, []);
  // One value for the provider's life: a new object each render would re-render
  // every row's actions on every message and every keystroke in the composer.
  const api = useMemo<TimelineCopyApi>(() => ({ copy }), [copy]);

  return (
    <TimelineCopyContext.Provider value={api}>
      {children}
      <div className="sr-only" role="status" aria-live="polite" aria-atomic="true">
        <span key={announcement.count}>{announcement.text}</span>
      </div>
    </TimelineCopyContext.Provider>
  );
}

/** The timeline's copy action; outside a provider it still copies, silently. */
export function useTimelineCopy(): TimelineCopyApi['copy'] {
  const api = useContext(TimelineCopyContext);
  return useCallback(
    async (text: string) => {
      if (api) return api.copy(text);
      try {
        await writeClipboard(text);
        return true;
      } catch {
        return false;
      }
    },
    [api]
  );
}

/** What a copy control shows: nothing yet, or the outcome of its last copy. */
export type CopyOutcome = 'copied' | 'failed' | null;

/** The outcome of this control's last copy, for two seconds. */
export function useCopyOutcome(): [CopyOutcome, (text: string) => Promise<void>] {
  const copy = useTimelineCopy();
  const [outcome, setOutcome] = useState<CopyOutcome>(null);
  const timer = useRef<number | null>(null);
  useEffect(
    () => () => {
      if (timer.current !== null) window.clearTimeout(timer.current);
    },
    []
  );
  const run = useCallback(
    async (text: string) => {
      const landed = await copy(text);
      setOutcome(landed ? 'copied' : 'failed');
      if (timer.current !== null) window.clearTimeout(timer.current);
      timer.current = window.setTimeout(() => setOutcome(null), COPY_FEEDBACK_MS);
    },
    [copy]
  );
  return [outcome, run];
}

/**
 * A glyph-only copy button. `label` is what its tooltip says at rest ("Copy text"); `name`,
 * when given, is its fuller accessible name ("Copy text of Bob Lee’s message, 10:02 AM"), which
 * contains the label. After a press the control itself answers: the glyph turns into a check
 * and the tooltip, held open, says "Copied" — or an X and "Couldn't copy".
 */
export function CopyIconButton({
  text,
  label,
  name,
  tabIndex,
  className,
}: {
  text: string;
  label: string;
  name?: string;
  tabIndex?: number;
  className?: string;
}) {
  const [outcome, copy] = useCopyOutcome();
  const [hovered, setHovered] = useState(false);
  const tip =
    outcome === 'copied'
      ? timelineCopy.copied
      : outcome === 'failed'
        ? timelineCopy.copyFailedShort
        : label;
  return (
    <Tooltip open={outcome !== null || hovered} onOpenChange={setHovered}>
      <TooltipTrigger asChild>
        <Button
          type="button"
          variant="ghost"
          size="sm"
          shape="round"
          aria-label={name ?? label}
          tabIndex={tabIndex}
          data-copy-outcome={outcome ?? undefined}
          className={cn('text-text-muted', className)}
          onClick={() => void copy(text)}
        >
          {outcome === 'copied' ? (
            <Check aria-hidden className="biorouter-check-settled" />
          ) : outcome === 'failed' ? (
            <X aria-hidden />
          ) : (
            <Copy aria-hidden />
          )}
        </Button>
      </TooltipTrigger>
      <TooltipContent>{tip}</TooltipContent>
    </Tooltip>
  );
}
