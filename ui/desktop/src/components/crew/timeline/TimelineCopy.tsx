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
import {
  DropdownMenuSeparator,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
} from '../../ui/dropdown-menu';
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

/** How long a menu stays open showing "Copied" before it closes itself (Q2-34). */
export const MENU_COPY_CLOSE_MS = 600;

/**
 * Copy items that answer inside their own menu (Q2-34): the chosen item reads "Copied" and the
 * menu stays open for {@link MENU_COPY_CLOSE_MS}, then closes; a refused copy reads "Couldn't
 * copy" for {@link COPY_FEEDBACK_MS} and the menu stays, so the person can try again or select
 * the text. A menu that closed on the click said nothing at all. `copy` announces the outcome
 * where the menu's owner announces it; `setOpen` is the menu's own open state.
 *
 * Pass `onOpenChange` as the menu's, and `select(item, text)` as a copy item's `onSelect`.
 */
export function useMenuCopy<Item extends string>(
  copy: (text: string) => Promise<boolean>,
  setOpen: (open: boolean) => void
) {
  const [outcome, setOutcome] = useState<{ item: Item; copied: boolean } | null>(null);
  const timer = useRef<number | null>(null);
  const clearTimer = useCallback(() => {
    if (timer.current !== null) window.clearTimeout(timer.current);
    timer.current = null;
  }, []);
  useEffect(() => clearTimer, [clearTimer]);

  const run = useCallback(
    async (item: Item, text: string) => {
      const copied = await copy(text);
      setOutcome({ item, copied });
      clearTimer();
      timer.current = window.setTimeout(
        () => {
          timer.current = null;
          // The label keeps "Copied" while the menu fades out; opening it again resets it.
          if (copied) setOpen(false);
          else setOutcome(null);
        },
        copied ? MENU_COPY_CLOSE_MS : COPY_FEEDBACK_MS
      );
    },
    [copy, setOpen, clearTimer]
  );

  const onOpenChange = useCallback(
    (next: boolean) => {
      clearTimer();
      if (next) setOutcome(null);
      setOpen(next);
    },
    [setOpen, clearTimer]
  );

  return {
    onOpenChange,
    /** A copy item's `onSelect`: keeps the menu open and copies. */
    select: (item: Item, text: string) => (event: Event) => {
      event.preventDefault();
      void run(item, text);
    },
    /** What the item says: its own words, or the outcome of the copy it just made. */
    label: (item: Item, idle: string) =>
      outcome?.item === item
        ? outcome.copied
          ? timelineCopy.copied
          : timelineCopy.copyFailedShort
        : idle,
    /** `data-crew-copy-state` for the item, while it shows an outcome. */
    state: (item: Item): 'copied' | 'failed' | undefined =>
      outcome?.item === item ? (outcome.copied ? 'copied' : 'failed') : undefined,
  };
}

/**
 * The one place a menu keeps its machine-ID copies (Q3-26): a separator, then a "Copy for support"
 * submenu holding those items and nothing else, as the LAST thing in the menu. A person's own
 * copies (Copy text, Copy channel name, Copy error) stay at the top level; an ID is what someone
 * reads out to support, so it is one deliberate step away and never the menu's first answer.
 *
 * The items inside answer as any copy item does (`useMenuCopy`): the chosen one reads "Copied"
 * until the whole menu closes, which unmounts this submenu with it.
 */
export function CopyForSupport({
  children,
  label = timelineCopy.copyForSupport,
}: {
  children: ReactNode;
  label?: string;
}) {
  return (
    <>
      <DropdownMenuSeparator />
      <DropdownMenuSub>
        <DropdownMenuSubTrigger data-crew-copy-for-support="">{label}</DropdownMenuSubTrigger>
        <DropdownMenuSubContent>{children}</DropdownMenuSubContent>
      </DropdownMenuSub>
    </>
  );
}

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
