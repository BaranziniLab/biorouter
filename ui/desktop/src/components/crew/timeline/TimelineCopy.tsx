import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from 'react';
import { Button } from '../../ui/button';
import { Tooltip, TooltipContent, TooltipTrigger } from '../../ui/Tooltip';
import { Check, Copy } from '../../icons/app-icons';
import { cn } from '../../../utils';
import { timelineCopy } from './copy';

/**
 * Copying from the timeline — a message's text, a code block, a message or task
 * ID, a task's error — confirms itself without a toast: the control that copied
 * shows a check for two seconds, and ONE polite region per timeline says
 * "Copied" (or how to copy by hand when the clipboard refuses). The region sits
 * outside the log, so an announcement never reads as a new message.
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

  return (
    <TimelineCopyContext.Provider value={{ copy }}>
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

/** A check for two seconds after a successful copy. */
export function useCopiedFlag(): [boolean, (text: string) => Promise<void>] {
  const copy = useTimelineCopy();
  const [copied, setCopied] = useState(false);
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
      if (!landed) return;
      setCopied(true);
      if (timer.current !== null) window.clearTimeout(timer.current);
      timer.current = window.setTimeout(() => setCopied(false), COPY_FEEDBACK_MS);
    },
    [copy]
  );
  return [copied, run];
}

/** A glyph-only copy button: the name is its tooltip, and the glyph turns into a check. */
export function CopyIconButton({
  text,
  label,
  tabIndex,
  className,
}: {
  text: string;
  label: string;
  tabIndex?: number;
  className?: string;
}) {
  const [copied, copy] = useCopiedFlag();
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button
          type="button"
          variant="ghost"
          size="sm"
          shape="round"
          aria-label={label}
          tabIndex={tabIndex}
          className={cn('text-text-muted', className)}
          onClick={() => void copy(text)}
        >
          {copied ? (
            <Check aria-hidden className="biorouter-check-settled" />
          ) : (
            <Copy aria-hidden />
          )}
        </Button>
      </TooltipTrigger>
      <TooltipContent>{label}</TooltipContent>
    </Tooltip>
  );
}
