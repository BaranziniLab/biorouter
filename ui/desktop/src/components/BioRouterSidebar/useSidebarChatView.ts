import { useCallback, useEffect, useMemo, useState } from 'react';

import {
  parseCollapsedFolders,
  parseSidebarChatView,
  readCollapsedFolders,
  readSidebarChatView,
  SIDEBAR_CHAT_VIEW_STORAGE_KEY,
  SIDEBAR_COLLAPSED_FOLDERS_STORAGE_KEY,
  writeCollapsedFolders,
  writeSidebarChatView,
  type SidebarChatView,
} from './sidebarChatView';

/**
 * The person's sidebar view (Group by, Sort by), remembered per viewer and kept
 * in step across windows: a change in one window arrives in the others through
 * the `storage` event, which fires only in the windows that did not write.
 */
export function useSidebarChatView(): [SidebarChatView, (view: SidebarChatView) => void] {
  const [view, setViewState] = useState<SidebarChatView>(() => readSidebarChatView());

  useEffect(() => {
    const onStorage = (event: StorageEvent) => {
      if (event.key !== SIDEBAR_CHAT_VIEW_STORAGE_KEY && event.key !== null) return;
      setViewState(
        event.key === null ? readSidebarChatView() : parseSidebarChatView(event.newValue)
      );
    };
    window.addEventListener('storage', onStorage);
    return () => window.removeEventListener('storage', onStorage);
  }, []);

  const setView = useCallback((next: SidebarChatView) => {
    setViewState(next);
    writeSidebarChatView(next);
  }, []);

  return [view, setView];
}

/** The folder groups the person collapsed, remembered the same way. */
export function useCollapsedFolders(): [ReadonlySet<string>, (folder: string) => void] {
  const [folders, setFolders] = useState<string[]>(() => readCollapsedFolders());

  useEffect(() => {
    const onStorage = (event: StorageEvent) => {
      if (event.key !== SIDEBAR_COLLAPSED_FOLDERS_STORAGE_KEY && event.key !== null) return;
      setFolders(
        event.key === null ? readCollapsedFolders() : parseCollapsedFolders(event.newValue)
      );
    };
    window.addEventListener('storage', onStorage);
    return () => window.removeEventListener('storage', onStorage);
  }, []);

  const toggle = useCallback((folder: string) => {
    setFolders((current) => {
      const next = current.includes(folder)
        ? current.filter((entry) => entry !== folder)
        : [...current, folder];
      writeCollapsedFolders(next);
      return next;
    });
  }, []);

  const collapsed = useMemo(() => new Set(folders), [folders]);
  return [collapsed, toggle];
}
