import { useEffect } from 'react';
import { subscribeSessionNameChanges } from '../../utils/sessionNameSync';
import { getWindowId, renameSavedTabs } from './chatGroupsStorage';

/**
 * Keep this window's saved tab titles on the name every other surface shows.
 *
 * Mounted ONCE, at the app root (`AppInner`), because that is the one place
 * that lives as long as the renderer. The tab store (`ChatGroupsProvider`) is
 * mounted with `/pair` only, so its own rename subscription missed every rename
 * made while another route showed: a chat renamed from its sidebar row on
 * History, Home or Settings came back to `/pair` under its old title, kept it
 * across a reload, and corrected only when its tab was made active (V2-R3-1).
 * One chat, one name, on every surface.
 *
 * Each rename (a person's, the auto-name, a rollback, a sibling window's) is
 * written into the saved record with `renameSavedTabs`, which the provider
 * reads when it mounts, so the strip and its overflow list open on the new name.
 * The provider still subscribes while it is mounted; that is what moves a title
 * on screen.
 */
export function useSavedTabTitleSync(): void {
  useEffect(
    () =>
      subscribeSessionNameChanges(({ sessionId, name, userSetName }) => {
        renameSavedTabs(getWindowId(), sessionId, name, userSetName);
      }),
    []
  );
}
