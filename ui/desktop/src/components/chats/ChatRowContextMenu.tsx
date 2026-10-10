import type { ReactNode } from 'react';

import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuTrigger,
  ContextMenuSeparator,
} from '../ui/context-menu';
import { DropdownMenuContent, DropdownMenuItem, DropdownMenuSeparator } from '../ui/dropdown-menu';
import { chatRowActions, type ChatRowActionTarget } from './chatRowActions';
import { chatRowCopy } from './copy';

/**
 * The menu on a chat row, wherever a chat row is drawn: the sidebar, History and
 * the tab strip ([#114](https://github.com/BaranziniLab/biorouter/issues/114),
 * owner message 4).
 *
 * One fixed order, text only (Crew's `ChannelRow` menus), with each group split
 * by a hairline:
 *
 *   Rename  F2
 *   ─
 *   Open in new tab · Open in new window
 *   ─
 *   Diverge · Export… · Copy chat ID · (a surface's own extra items)
 *   ─
 *   Delete chat…
 *
 * The three shared actions still come from `chatRowActions`, whose list is
 * pinned by four tests; everything else is an optional handler a surface passes
 * when its chat allows it (History's `⋯` menu is the reference for which items a
 * subagent, scheduled or terminal chat offers). A handler that is absent leaves
 * its item out, so a surface never shows an action it cannot run.
 */
export interface ChatRowMenuHandlers {
  /** Start renaming the chat in place (the sidebar) or in the surface's own editor. */
  onRename?: () => void;
  /** The hint shown beside Rename; `null` hides it on a surface without the key. */
  renameShortcut?: string | null;
  onDiverge?: () => void;
  onExport?: () => void;
  onDelete?: () => void;
  /** A surface's own items, placed right after Copy chat ID (History's "Make this chat public"). */
  extraItems?: readonly ChatRowMenuExtraItem[];
}

export interface ChatRowMenuExtraItem {
  key: string;
  label: string;
  onSelect: () => void;
  disabled?: boolean;
}

export type ChatRowMenuEntry =
  | {
      kind: 'item';
      key: string;
      label: string;
      onSelect: () => void;
      danger?: boolean;
      shortcut?: string;
      disabled?: boolean;
    }
  | { kind: 'separator'; key: string };

/** The ordered entries of a chat row's menu, for any menu primitive to draw. */
export function chatRowMenuEntries(
  target: ChatRowActionTarget,
  handlers: ChatRowMenuHandlers = {}
): ChatRowMenuEntry[] {
  const entries: ChatRowMenuEntry[] = [];
  const [openTab, openWindow, copyId] = chatRowActions(target);

  if (handlers.onRename) {
    const shortcut =
      handlers.renameShortcut === undefined
        ? chatRowCopy.menu.renameShortcut
        : (handlers.renameShortcut ?? undefined);
    entries.push({
      kind: 'item',
      key: 'rename',
      label: chatRowCopy.menu.rename,
      onSelect: handlers.onRename,
      ...(shortcut ? { shortcut } : {}),
    });
    entries.push({ kind: 'separator', key: 'after-rename' });
  }

  for (const action of [openTab, openWindow]) {
    entries.push({ kind: 'item', key: action.key, label: action.label, onSelect: action.run });
  }
  entries.push({ kind: 'separator', key: 'after-open' });

  if (handlers.onDiverge) {
    entries.push({
      kind: 'item',
      key: 'diverge',
      label: chatRowCopy.menu.diverge,
      onSelect: handlers.onDiverge,
    });
  }
  if (handlers.onExport) {
    entries.push({
      kind: 'item',
      key: 'export',
      label: chatRowCopy.menu.export,
      onSelect: handlers.onExport,
    });
  }
  entries.push({ kind: 'item', key: copyId.key, label: copyId.label, onSelect: copyId.run });
  for (const extra of handlers.extraItems ?? []) {
    entries.push({ kind: 'item', ...extra });
  }

  if (handlers.onDelete) {
    entries.push({ kind: 'separator', key: 'before-delete' });
    entries.push({
      kind: 'item',
      key: 'delete',
      label: chatRowCopy.menu.delete,
      onSelect: handlers.onDelete,
      danger: true,
    });
  }
  return entries;
}

/** The trailing hint on a menu row (Rename's F2): small, muted, at the row's end. */
function MenuShortcut({ children }: { children: ReactNode }) {
  return (
    <span aria-hidden="true" className="ml-auto text-supporting text-text-subtle">
      {children}
    </span>
  );
}

/**
 * The right-click menu's content. A surface that already composes its own
 * trigger (a sidebar row inside a `TooltipTrigger asChild`) renders this inside
 * its `ContextMenu`; everyone else uses {@link ChatRowContextMenu}.
 */
export function ChatRowContextMenuContent({
  target,
  ...handlers
}: { target: ChatRowActionTarget } & ChatRowMenuHandlers) {
  return (
    <ContextMenuContent className="w-56">
      {chatRowMenuEntries(target, handlers).map((entry) =>
        entry.kind === 'separator' ? (
          <ContextMenuSeparator key={entry.key} />
        ) : (
          <ContextMenuItem
            key={entry.key}
            data-chat-row-action={entry.key}
            onSelect={entry.onSelect}
            disabled={entry.disabled}
            data-variant={entry.danger ? 'destructive' : undefined}
            className={entry.danger ? 'text-text-danger' : undefined}
          >
            {entry.label}
            {entry.shortcut ? <MenuShortcut>{entry.shortcut}</MenuShortcut> : null}
          </ContextMenuItem>
        )
      )}
    </ContextMenuContent>
  );
}

/**
 * The same menu for a `⋯` button (a `DropdownMenu`), so the overflow and the
 * right-click menu can never list different things.
 */
export function ChatRowDropdownMenuContent({
  target,
  align = 'end',
  ...handlers
}: { target: ChatRowActionTarget; align?: 'start' | 'center' | 'end' } & ChatRowMenuHandlers) {
  return (
    <DropdownMenuContent align={align} className="w-56">
      {chatRowMenuEntries(target, handlers).map((entry) =>
        entry.kind === 'separator' ? (
          <DropdownMenuSeparator key={entry.key} />
        ) : (
          <DropdownMenuItem
            key={entry.key}
            data-chat-row-action={entry.key}
            onSelect={entry.onSelect}
            disabled={entry.disabled}
            variant={entry.danger ? 'destructive' : 'default'}
          >
            {entry.label}
            {entry.shortcut ? <MenuShortcut>{entry.shortcut}</MenuShortcut> : null}
          </DropdownMenuItem>
        )
      )}
    </DropdownMenuContent>
  );
}

/**
 * Wrap a row in its right-click menu. `children` must be a single DOM element —
 * the trigger attaches to it with `asChild`, so the row's own layout, ref and
 * handlers are untouched.
 */
export function ChatRowContextMenu({
  target,
  children,
  ...handlers
}: {
  target: ChatRowActionTarget;
  children: ReactNode;
} & ChatRowMenuHandlers) {
  return (
    <ContextMenu>
      <ContextMenuTrigger asChild>{children}</ContextMenuTrigger>
      <ChatRowContextMenuContent target={target} {...handlers} />
    </ContextMenu>
  );
}
