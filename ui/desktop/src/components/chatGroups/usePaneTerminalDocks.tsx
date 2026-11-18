import { useCallback, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import type {
  TerminalDockContextValue,
  TerminalDockEntry,
} from '../../contexts/TerminalDockContext';
import InAppTerminalDock from '../InAppTerminalDock';
import { leafGroupIds, type ChatGroupId, type ChatGroupsState } from './chatGroupsTypes';

interface TerminalOwner {
  groupId: ChatGroupId;
  activeKey: string;
}

interface TerminalMount {
  container: HTMLDivElement;
  focused: HTMLElement | null;
}

type RegisterMount = (key: string, mount: TerminalMount) => () => void;

function PaneTerminalDock({
  terminal,
  owner,
  registerMount,
  onFocusGroup,
  onClose,
  onEmptied,
}: {
  terminal: TerminalDockEntry;
  owner: TerminalOwner | undefined;
  registerMount: RegisterMount;
  onFocusGroup: (groupId: ChatGroupId) => void;
  onClose: () => void;
  onEmptied: () => void;
}) {
  const [mount] = useState<TerminalMount>(() => {
    const container = document.createElement('div');
    container.className = 'contents';
    return { container, focused: null };
  });

  useLayoutEffect(() => registerMount(terminal.key, mount), [terminal.key, mount, registerMount]);

  const focusGroup = () => {
    if (owner) onFocusGroup(owner.groupId);
  };

  return createPortal(
    <div className="contents" onPointerDownCapture={focusGroup} onFocusCapture={focusGroup}>
      <InAppTerminalDock
        dockKey={terminal.key}
        open={terminal.showing && owner?.activeKey === terminal.key}
        workingDir={terminal.workingDir}
        onClose={onClose}
        onEmptied={onEmptied}
      />
    </div>,
    mount.container
  );
}

/** A tab's terminal survives changes to the pane tree that contains its DOM host. */
export function usePaneTerminalDocks(
  state: ChatGroupsState | undefined,
  terminalDock: TerminalDockContextValue | null,
  onFocusGroup: (groupId: ChatGroupId) => void
) {
  const hosts = useRef(new Map<ChatGroupId, HTMLDivElement>());
  const mounts = useRef(new Map<string, TerminalMount>());
  const owners = useMemo(() => {
    const next = new Map<string, TerminalOwner>();
    if (!state) return next;
    for (const groupId of leafGroupIds(state.layout)) {
      const group = state.groups[groupId];
      if (!group) continue;
      const emptyKey = `${groupId}-empty`;
      const owner = { groupId, activeKey: group.activeTabId ?? emptyKey };
      for (const key of [...group.tabs.map((tab) => tab.tabId), emptyKey]) next.set(key, owner);
    }
    return next;
  }, [state]);
  const ownersRef = useRef(owners);
  ownersRef.current = owners;

  const registerHost = useCallback((groupId: ChatGroupId, host: HTMLDivElement | null) => {
    const previous = hosts.current.get(groupId);
    const active = document.activeElement;
    // A deleted pane's ref detaches before its DOM does. The parent layout effect
    // can then move the same focused terminal instead of losing focus with that pane.
    if (previous && previous !== host && active instanceof HTMLElement) {
      for (const mount of mounts.current.values()) {
        if (previous.contains(mount.container) && mount.container.contains(active)) {
          mount.focused = active;
        }
      }
    }
    if (host) hosts.current.set(groupId, host);
    else hosts.current.delete(groupId);
  }, []);

  const registerMount = useCallback<RegisterMount>((key, mount) => {
    mounts.current.set(key, mount);
    const owner = ownersRef.current.get(key);
    const host = owner && hosts.current.get(owner.groupId);
    // StrictMode can replay a newly opened dock without replaying the Shell's
    // layout effect. Its stable container must reconnect during registration too.
    if (host && mount.container.parentNode !== host) host.appendChild(mount.container);
    return () => {
      if (mounts.current.get(key) === mount) mounts.current.delete(key);
      mount.container.remove();
    };
  }, []);

  // All pane host refs are attached before this parent layout effect. Moving the
  // stable portal container here keeps its dock mounted and inside compact inert
  // ancestry before xterm's passive initialization or ResizeObserver delivery.
  useLayoutEffect(() => {
    for (const terminal of terminalDock?.terminals ?? []) {
      const owner = owners.get(terminal.key);
      const host = owner && hosts.current.get(owner.groupId);
      const mount = mounts.current.get(terminal.key);
      if (!host || !mount) continue;
      if (mount.container.parentNode === host) {
        mount.focused = null;
        continue;
      }
      const active = document.activeElement;
      const focused =
        active instanceof HTMLElement && mount.container.contains(active) ? active : mount.focused;
      host.appendChild(mount.container);
      mount.focused = null;
      if (
        focused?.isConnected &&
        terminal.showing &&
        owner.activeKey === terminal.key &&
        !focused.closest('[inert]')
      ) {
        focused.focus({ preventScroll: true });
      }
    }
  });

  return {
    registerHost,
    docks: (terminalDock?.terminals ?? []).map((terminal) => (
      <PaneTerminalDock
        key={terminal.key}
        terminal={terminal}
        owner={owners.get(terminal.key)}
        registerMount={registerMount}
        onFocusGroup={onFocusGroup}
        onClose={() => terminalDock?.setOpen(terminal.key, false)}
        onEmptied={() => terminalDock?.remove(terminal.key)}
      />
    )),
  };
}
