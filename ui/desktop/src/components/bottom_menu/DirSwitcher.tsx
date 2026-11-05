import './pickers.css';
import React, { useId, useState } from 'react';
import { Folder } from '../icons/app-icons';
import { updateWorkingDir } from '../../api';
import { userActionHeaders } from '../../utils/userAction';
import { toastError } from '../../toasts';
import { ChatState } from '../../types/chatState';
import { FOOTER_COPY } from './copy';
import { FolderAccessCard } from './FolderAccessCard';
import { FooterCardRoot, FooterCardTrigger } from './FooterCard';
import { useFooterCard } from './useFooterCard';

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
   * empty. Once the chat has messages, the card offers no Choose folder…: it
   * says the folder is fixed instead, and nothing here can reach the chooser
   * or `updateWorkingDir`. The backend enforces the same rule with a 409, so
   * this is UX, not the guard.
   */
  locked?: boolean;
  onWorkingDirChange?: (newDir: string) => void;
  onRestartStart?: () => void;
  onRestartEnd?: () => void;
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
 * The composer footer's working-folder chip (spec 3.7) and its card, one of the
 * footer's three items that behave alike (`useFooterCard`, `FooterCard`).
 *
 * The chip names the folder; hovering it (300ms), reaching it with Tab, or
 * clicking it opens `FolderAccessCard`: the full path, Choose folder… while the
 * chat is empty, every other folder the chat may use, and Revoke for each. It
 * replaced the plain path tooltip and the separate "Folder access" button, so
 * the only control that revokes a folder grant lives here. The card hangs above
 * the composer card (the footer's `FooterCardGroup`), never over the draft.
 *
 * - The click (or tap, Enter, Space) pins the card in EVERY state: Home, an
 *   empty chat, a chat with messages and a folder not known yet. A keyboard
 *   press also moves focus into the card.
 * - Choosing a folder is the card's Choose folder…, offered while the chat is
 *   empty and its folder is known. It closes the card and opens the native
 *   chooser, and the card stays shut while the chooser is open (#50: nothing
 *   stuck open over a native dialog, nothing armed during it opens after).
 * - Locked (#44, the chat has messages): the card says the folder is fixed,
 *   and nothing here opens the chooser or calls `updateWorkingDir`.
 * - Cmd/Ctrl-click opens a known folder in the file manager, in any state (an
 *   accelerator: it neither pins the card nor changes the folder).
 * - Folder not known yet (`workingDir` null, an existing chat): the folder
 *   glyph and no name, and the same card, so Revoke never depends on the
 *   folder's row having answered.
 */
export const DirSwitcher: React.FC<DirSwitcherProps> = ({
  className,
  sessionId,
  workingDir,
  locked = false,
  onWorkingDirChange,
  onRestartStart,
  onRestartEnd,
}) => {
  const [isDirectoryChooserOpen, setIsDirectoryChooserOpen] = useState(false);
  const card = useFooterCard({ suppressed: isDirectoryChooserOpen });
  const cardId = useId();

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

  // #44: only an empty chat with a known folder may choose another, and only
  // from the card. The locked chip passes nothing, so its card has no button.
  const chooseFolder =
    !locked && workingDir !== null
      ? () => {
          card.close();
          void handleDirectoryChange();
        }
      : undefined;

  /** Runs before the pin: the accelerators, which never pin the card. */
  const onChipClick = (event: React.MouseEvent) => {
    // The native chooser is up: a click arms nothing for after it (#50).
    if (isDirectoryChooserOpen) {
      event.preventDefault();
      return;
    }
    if ((event.metaKey || event.ctrlKey) && workingDir !== null) {
      event.preventDefault();
      card.close();
      void window.electron.openDirectoryInExplorer(workingDir);
    }
  };

  // The footer shows the folder's NAME in its sans (spec 3.7, principle 10:
  // names, not paths); the full path is in the card, in mono, because a path is
  // a machine string. `<bdi>` keeps a right-to-left folder name from reordering
  // the line around it.
  const name =
    workingDir === null ? null : (
      <span className="br-footline__name">
        <bdi>{workingDirLabel(workingDir)}</bdi>
      </span>
    );

  // #50: the Popover is CONTROLLED for the component's whole lifetime
  // (`FooterCardRoot`). Locking changes only what the card says, never the
  // trigger or the root, and `useFooterCard` keeps the card shut while the
  // native chooser is open.
  return (
    <FooterCardRoot card={card}>
      <FooterCardTrigger
        card={card}
        cardId={cardId}
        className={className}
        data-testid={
          workingDir === null
            ? 'dir-switcher-unknown'
            : locked
              ? 'dir-switcher-locked'
              : 'dir-switcher'
        }
        // The chip's name carries the full path, so a screen reader has it
        // without opening the card.
        aria-label={
          workingDir === null ? FOOTER_COPY.folderUnknownLabel : FOOTER_COPY.folderLabel(workingDir)
        }
        onClick={onChipClick}
      >
        <Folder aria-hidden="true" />
        {name}
      </FooterCardTrigger>
      <FolderAccessCard
        id={cardId}
        sessionId={sessionId}
        workingDir={workingDir}
        locked={locked}
        onChooseFolder={chooseFolder}
        card={card}
      />
    </FooterCardRoot>
  );
};
