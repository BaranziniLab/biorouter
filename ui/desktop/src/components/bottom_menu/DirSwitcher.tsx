import './pickers.css';
import React, { useId, useState } from 'react';
import { Folder } from '../icons/app-icons';
import { Popover, PopoverAnchor } from '../ui/popover';
import { cn } from '../../utils';
import { updateWorkingDir } from '../../api';
import { userActionHeaders } from '../../utils/userAction';
import { toastError } from '../../toasts';
import { ChatState } from '../../types/chatState';
import { FOOTER_COPY } from './copy';
import { FolderAccessCard } from './FolderAccessCard';
import { useFolderCard } from './useFolderCard';

interface DirSwitcherProps {
  /** Layout only. */
  className?: string;
  sessionId: string | undefined;
  /**
   * The chat's working folder, or `null` while an existing chat's folder is not
   * known yet (V2-R7-3). The chip then names no folder (the folder glyph alone)
   * but still opens the card, so the chat's other folders and their Revoke stay
   * reachable while the folder is unknown.
   */
  workingDir: string | null;
  /**
   * #44: the working directory is choosable only while the chat is completely
   * empty. Once the chat has messages, the chip no longer opens the chooser:
   * its click pins the folder card open instead (basename on the chip, full
   * path and folder access in the card). The backend enforces the same rule
   * with a 409, so this is UX, not the guard.
   */
  locked?: boolean;
  onWorkingDirChange?: (newDir: string) => void;
  onRestartStart?: () => void;
  onRestartEnd?: () => void;
  /**
   * The composer card the chip sits under. The folder card hangs above it, so
   * a card that a hover opened never covers the draft, `+` or Tools; without
   * it the card hangs from the chip.
   */
  composerRef?: React.RefObject<HTMLElement | null>;
}

/**
 * The short display name for a working directory: its basename ("Desktop" for
 * /Users/wgu/Desktop). A filesystem root ("/", "C:\") has no basename and is
 * shown as-is; the home directory shows its own basename (e.g. "wgu"), which
 * stays unambiguous alongside the full path in the chip's card.
 */
export function workingDirLabel(dir: string): string {
  // A Windows drive root ("C:\", "C:/") is a root, not a folder named "C:".
  if (/^[A-Za-z]:[\\/]+$/.test(dir)) return dir;
  const segments = dir.split(/[/\\]+/).filter(Boolean);
  return segments.length > 0 ? segments[segments.length - 1] : dir;
}

/**
 * #44 — whether the folder chip must render read-only, derived from
 * authoritative state rather than the loaded transcript's length alone.
 * `messages.length` misleads in exactly the two ways this function fixes:
 * it is 0 while a resumed transcript is still hydrating (which would briefly
 * unlock a non-empty session), and it is >0 after a FAILED first submit whose
 * optimistic message is retained by design in the transcript so it can go out
 * when the agent lands (which would lock a session the server still considers
 * empty). The server's 409 remains the real guard; this is the UX mirror.
 */
export function deriveWorkingDirLocked(params: {
  sessionId: string | null | undefined;
  /**
   * `message_count` from the last session fetch — the server's own word on
   * whether the chat has messages. `undefined` while the session metadata is
   * still loading.
   */
  persistedMessageCount: number | undefined;
  /** Any assistant message in the transcript proves a message reached the server. */
  hasAssistantMessage: boolean;
  chatState: ChatState;
}): boolean {
  const { sessionId, persistedMessageCount, hasAssistantMessage, chatState } = params;
  // Pre-session (#39): the chooser is always available.
  if (!sessionId) return false;
  // Until the session metadata arrives, assume locked: a resumed session must
  // be locked from first paint until history proves empty.
  if (persistedMessageCount === undefined) return true;
  if (persistedMessageCount > 0) return true;
  // The fetched count can be stale during a live conversation: an assistant
  // reply, or a turn actively in flight, means the first message has reached
  // (or is reaching) the server even though the last fetch said zero.
  if (hasAssistantMessage) return true;
  if (chatState !== ChatState.Idle && chatState !== ChatState.LoadingConversation) return true;
  // Zero persisted messages, no reply, no turn in flight: any message still in
  // the transcript is an optimistic one from a failed submit — the dir is
  // still choosable.
  return false;
}

/**
 * The composer footer's working-folder chip (spec 3.7) and its card.
 *
 * The chip names the folder; hovering it (300ms), or reaching it with Tab, opens
 * `FolderAccessCard`: the full path, every other folder the chat may use, and
 * Revoke for each. It replaced the plain path tooltip and the separate "Folder
 * access" button, so the only control that revokes a folder grant lives here.
 * The card hangs above the composer card (`composerRef`), never over the draft.
 *
 * - Unlocked (an empty chat, or Home): the click opens the native folder
 *   chooser and Cmd/Ctrl-click opens the folder in the file manager, as before.
 *   Either click closes the card, and it stays shut while the chooser is open
 *   (#50: nothing stuck open over a native dialog).
 * - Locked (#44, the chat has messages): a real button whose click, Enter,
 *   Space or tap pins the card open (a keyboard press also moves focus into
 *   it). It never opens the chooser and never calls `updateWorkingDir`.
 * - Folder not known yet (`workingDir` null, an existing chat): the same
 *   card-only button with the folder glyph and no name, so Revoke never
 *   depends on the folder's row having answered.
 */
export const DirSwitcher: React.FC<DirSwitcherProps> = ({
  className,
  sessionId,
  workingDir,
  locked = false,
  onWorkingDirChange,
  onRestartStart,
  onRestartEnd,
  composerRef,
}) => {
  const [isDirectoryChooserOpen, setIsDirectoryChooserOpen] = useState(false);
  const card = useFolderCard({ suppressed: isDirectoryChooserOpen, clearOf: composerRef });
  const cardId = useId();
  // The card is the chip's only action once the chat has messages (#44), and
  // while its folder is not known: there is no folder to choose from or open.
  const cardOnly = locked || workingDir === null;

  const handleDirectoryChange = async () => {
    if (isDirectoryChooserOpen) return;
    setIsDirectoryChooserOpen(true);

    let result;
    try {
      result = await window.electron.directoryChooser();
    } finally {
      setIsDirectoryChooserOpen(false);
    }

    if (result.canceled || result.filePaths.length === 0) {
      return;
    }

    const newDir = result.filePaths[0];

    if (sessionId) {
      onRestartStart?.();

      try {
        // `throwOnError` is load-bearing: the generated client otherwise
        // RESOLVES with an error object on a non-2xx (e.g. the #44 409 once
        // the chat has messages) and the catch below would never fire.
        await updateWorkingDir({
          // Issue #56 Task 58: repointing a private chat's working directory
          // needs the proof-of-user.
          headers: await userActionHeaders(),
          body: { session_id: sessionId, working_dir: newDir },
          throwOnError: true,
        });
        // Reflect the change only after the server accepted it, so a refusal
        // leaves the displayed dir and the recents list untouched.
        window.electron.addRecentDir(newDir);
        onWorkingDirChange?.(newDir);
      } catch (error) {
        console.error('[DirSwitcher] Failed to update working directory:', error);
        const serverMessage =
          typeof error === 'object' &&
          error !== null &&
          'message' in error &&
          typeof (error as { message?: unknown }).message === 'string'
            ? (error as { message: string }).message
            : undefined;
        toastError({
          title: FOOTER_COPY.folderUpdateFailedTitle,
          msg: serverMessage ?? FOOTER_COPY.folderUpdateFailed,
        });
      } finally {
        onRestartEnd?.();
      }
    } else {
      window.electron.addRecentDir(newDir);
      onWorkingDirChange?.(newDir);
    }
  };

  const handleDirectoryClick = async (event: React.MouseEvent) => {
    if (isDirectoryChooserOpen) {
      event.preventDefault();
      event.stopPropagation();
      return;
    }
    // The person acted on the folder itself: the card gives way, as the old
    // tooltip did on a trigger click.
    card.close();
    const isCmdOrCtrlClick = event.metaKey || event.ctrlKey;

    if (isCmdOrCtrlClick) {
      event.preventDefault();
      event.stopPropagation();
      if (workingDir !== null) await window.electron.openDirectoryInExplorer(workingDir);
    } else {
      await handleDirectoryChange();
    }
  };

  // Both chips show the folder's NAME in the footer's sans (spec 3.7, principle
  // 10: names, not paths); the full path is in the card, in mono, because a
  // path is a machine string. `<bdi>` keeps a right-to-left folder name from
  // reordering the line around it.
  const name =
    workingDir === null ? null : (
      <span className="br-footline__name">
        <bdi>{workingDirLabel(workingDir)}</bdi>
      </span>
    );
  const shared = {
    ref: card.triggerRef,
    className: cn('br-footline__item', className),
    // The chip's name carries the full path, so a screen reader has it without
    // opening the card.
    'aria-label':
      workingDir === null ? FOOTER_COPY.folderUnknownLabel : FOOTER_COPY.folderLabel(workingDir),
    ...card.triggerHandlers,
  };
  // #44: once the chat has messages the working dir is immutable, so the
  // locked chip has no chooser affordance at all: it only holds the card open.
  const trigger = cardOnly ? (
    <button
      type="button"
      data-testid={workingDir === null ? 'dir-switcher-unknown' : 'dir-switcher-locked'}
      aria-haspopup="dialog"
      aria-expanded={card.open}
      aria-controls={card.open ? cardId : undefined}
      // `detail` is 0 for Enter, Space and an assistive technology's click: those
      // move focus into the card, so Revoke can be reached from the keyboard.
      onClick={(event) => card.togglePin({ moveFocus: event.detail === 0 })}
      {...shared}
    >
      <Folder aria-hidden="true" />
      {name}
    </button>
  ) : (
    <button
      type="button"
      data-testid="dir-switcher"
      onClick={handleDirectoryClick}
      disabled={isDirectoryChooserOpen}
      {...shared}
    >
      <Folder aria-hidden="true" />
      {name}
    </button>
  );

  // #50: the Popover is CONTROLLED for the component's whole lifetime. Locking
  // swaps the trigger in place, so React reconciles both states into the same
  // root, and a root that flipped from controlled to uncontrolled mid-life is
  // what showed up as a stuck-open tooltip. `useFolderCard` also keeps the card
  // shut while the native chooser is open.
  //
  // The anchor is VIRTUAL: the chip stretched up to the composer card's top
  // (`folderCardAnchorRect`), so `side="top"` hangs the card above the whole
  // composer instead of over its left half.
  return (
    <Popover open={card.open} onOpenChange={card.onOpenChange}>
      <PopoverAnchor virtualRef={card.anchor} />
      {trigger}
      <FolderAccessCard
        id={cardId}
        sessionId={sessionId}
        workingDir={workingDir}
        locked={locked}
        card={card}
      />
    </Popover>
  );
};
