import './pickers.css';
import React, { useEffect, useId, useState } from 'react';
import { Folder } from '../icons/app-icons';
import { updateWorkingDir } from '../../api';
import { cn } from '../../utils';
import { userActionHeaders } from '../../utils/userAction';
import { folderName, homeFolder, tildePath } from '../../utils/folderPath';
import { isBrowserSurface } from '../../utils/surface';
import { toastError } from '../../toasts';
import { ChatState } from '../../types/chatState';
import { PathSegments } from '../ui/path-segments';
import { Tooltip, TooltipContent, TooltipTrigger } from '../ui/Tooltip';
import { useSurfaceTriggerHint } from '../ui/surfaceTriggerHint';
import { FOOTER_COPY } from './copy';
import { FolderAccessCard } from './FolderAccessCard';
import { FooterCardRoot } from './FooterCard';
import { useFooterCard } from './useFooterCard';

interface DirSwitcherProps {
  /** Layout only. */
  className?: string;
  sessionId: string | undefined;
  /**
   * The chat's working folder, or `null` while an existing chat's folder is not
   * known yet (V2-R7-3). The chip then names no folder (the folder glyph alone)
   * but its click still opens the folder access card, so the chat's other
   * folders and their Revoke stay reachable while the folder is unknown.
   */
  workingDir: string | null;
  /**
   * #44: the working directory is choosable only while the chat is completely
   * empty. Once the chat has messages, a click opens the folder access card
   * instead of the chooser, the tooltip says the folder is fixed, and nothing
   * here can reach the chooser or `updateWorkingDir`. The backend enforces the
   * same rule with a 409, so this is UX, not the guard.
   */
  locked?: boolean;
  onWorkingDirChange?: (newDir: string) => void;
  onRestartStart?: () => void;
  onRestartEnd?: () => void;
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
 * The composer footer's working-folder chip (spec 3.7). It is a CONTROL, like the model chip on
 * the same line, not a readout like the context ring and the cost (owner, 2026-11-19): a hover
 * names it and a click does its one job, with no card between the two.
 *
 * - A hover, or a Tab, shows the app's label tooltip (`Tooltip`, the model chip's and the
 *   microphone's): "Working folder: ~/Desktop/project", written as the app writes a folder
 *   (`tildePath`), then one short line of status.
 * - While the folder can still change (Home, a chat with no messages yet, the folder known), the
 *   click opens the native folder chooser directly. While the chooser is up the tooltip stays shut
 *   and a click arms nothing for after it (#50).
 * - Once the chat has messages the folder is fixed (#44): nothing here opens the chooser or calls
 *   `updateWorkingDir` (the daemon refuses it with a 409 too, so this is UX, not the guard). The
 *   tooltip says the folder is fixed, and the click opens the chat's folder access card
 *   (`FolderAccessCard`), which lists every other folder the chat may use with Revoke for each.
 *   That card is the desktop's and the browser's only way to revoke a folder grant
 *   (docs/security/folder-access.md), and only a chat that has had a turn can hold a grant, so the
 *   chip that cannot choose a folder is the one that opens it. It opens on a click, Enter or Space
 *   only, never on a hover, hangs above the composer card, and is one of the footer's cards
 *   (`FooterCardGroup`: one card at a time).
 * - Folder not known yet (`workingDir` null, an existing chat): the folder glyph and no name, the
 *   tooltip says so, and the click opens the same card, so Revoke never waits on the folder.
 * - Cmd-click on macOS, Ctrl-click on Windows and Linux, opens a known folder in the file manager,
 *   in any state, and does nothing else. It is the only modifier click in the interface. In a
 *   `biorouter serve` browser there is no file manager to open (`openDirectoryInExplorer` is a
 *   no-op there), so it is a plain click.
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
  // #44: only an empty chat with a known folder may choose another.
  const choosable = !locked && workingDir !== null;
  // The label tooltip names the chip on a hover, closes on the press, and never comes back over
  // the chooser or the card the click opened (H2).
  const hint = useSurfaceTriggerHint(isDirectoryChooserOpen || card.open);
  const closeCard = card.close;

  // A chip that becomes choosable (another, empty chat in the same composer) has no card to show.
  useEffect(() => {
    if (choosable) closeCard();
  }, [choosable, closeCard]);

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

  const onChipClick = (event: React.MouseEvent) => {
    // The native chooser is up: a click arms nothing for after it (#50).
    if (isDirectoryChooserOpen) return;
    // macOS's Ctrl-click is a secondary click, so the chord is Cmd there.
    const revealChord = window.electron?.platform === 'darwin' ? event.metaKey : event.ctrlKey;
    if (revealChord && workingDir !== null && !isBrowserSurface()) {
      card.close();
      void window.electron.openDirectoryInExplorer(workingDir);
      return;
    }
    if (choosable) {
      void handleDirectoryChange();
      return;
    }
    // `detail` is 0 for Enter, Space and an assistive technology's click: those also move focus
    // into the card, so its controls can be reached from the keyboard.
    card.togglePin({ moveFocus: event.detail === 0 });
  };

  // The footer shows the folder's NAME (spec 3.7, principle 10: names, not
  // paths), the word the sidebar's folder groups use (`~` for home, `Crew` for
  // the Crew task folder); the tooltip writes where it is, ending in this same
  // name. `<bdi>` keeps a right-to-left folder name from reordering the line
  // around it.
  const shownName = workingDir === null ? null : folderName(workingDir, homeFolder());
  const name =
    shownName === null ? null : (
      <span className="br-footline__name">
        <bdi>{shownName}</bdi>
      </span>
    );
  const status = choosable
    ? FOOTER_COPY.folderChangeHint
    : locked
      ? FOOTER_COPY.folderFixedHint
      : null;

  // #50: the Popover root and the tooltip are both CONTROLLED for the component's whole lifetime
  // (`FooterCardRoot`, `useSurfaceTriggerHint`). Locking changes what a click does and what the
  // tooltip says, never the trigger or either root.
  return (
    <FooterCardRoot card={card}>
      <Tooltip {...hint.tooltip}>
        <TooltipTrigger asChild {...hint.trigger}>
          <button
            type="button"
            ref={card.triggerRef}
            className={cn('br-footline__item', className)}
            data-testid={
              workingDir === null
                ? 'dir-switcher-unknown'
                : locked
                  ? 'dir-switcher-locked'
                  : 'dir-switcher'
            }
            // The chip's name carries the full path, so a screen reader has it
            // without the tooltip, and the word the chip shows (WCAG 2.5.3).
            aria-label={
              workingDir === null
                ? FOOTER_COPY.folderUnknownLabel
                : FOOTER_COPY.folderLabel(workingDir, shownName ?? undefined)
            }
            // A chip that chooses opens the system's own dialog, not one in the page; the one
            // that cannot choose opens the folder access card.
            aria-haspopup={choosable ? undefined : 'dialog'}
            aria-expanded={choosable ? undefined : card.open}
            aria-controls={!choosable && card.open ? cardId : undefined}
            data-pinned={card.pinned ? 'true' : undefined}
            onClick={onChipClick}
          >
            <Folder aria-hidden="true" />
            {name}
          </button>
        </TooltipTrigger>
        {/* On the chip's left edge, as its card is: the chip starts the line, so a centred tooltip
            reached back over the sidebar. */}
        <TooltipContent side="top" align="start" data-testid="dir-switcher-tooltip">
          <p>
            {workingDir === null ? (
              FOOTER_COPY.folderUnknownLabel
            ) : (
              <>
                {FOOTER_COPY.folderTooltipLabel}{' '}
                <span dir="ltr" className="br-folder-tip__path">
                  <PathSegments path={tildePath(workingDir, homeFolder())} />
                </span>
              </>
            )}
          </p>
          {status && <p>{status}</p>}
        </TooltipContent>
      </Tooltip>
      <FolderAccessCard
        id={cardId}
        sessionId={sessionId}
        workingDir={workingDir}
        locked={locked}
        card={card}
      />
    </FooterCardRoot>
  );
};
