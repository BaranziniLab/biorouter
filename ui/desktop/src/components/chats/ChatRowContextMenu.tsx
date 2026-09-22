import type { ReactNode } from 'react';

import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuTrigger,
  ContextMenuSeparator,
} from '../ui/context-menu';
import { Trash2 } from '../icons/app-icons';
import { chatRowActions, type ChatRowActionTarget } from './chatRowActions';

/**
 * The right-click menu on a chat row, wherever a chat row is drawn
 * ([#114](https://github.com/BaranziniLab/biorouter/issues/114)).
 *
 * Two exports rather than one, because one of the three surfaces cannot use the
 * wrapper: a sidebar Recents row is already inside a `TooltipTrigger asChild`,
 * so its trigger has to be composed by hand. `ChatRowContextMenuContent` is
 * what both paths share, so the item list, order and handlers stay single-source
 * either way.
 */
export function ChatRowContextMenuContent({
  target,
  onDelete,
}: {
  target: ChatRowActionTarget;
  onDelete?: () => void;
}) {
  return (
    <ContextMenuContent className="w-56">
      {chatRowActions(target).map(({ key, label, icon: Icon, run }) => (
        <ContextMenuItem key={key} onSelect={run}>
          <Icon className="h-4 w-4" />
          {label}
        </ContextMenuItem>
      ))}
      {onDelete && (
        <>
          <ContextMenuSeparator />
          <ContextMenuItem onSelect={onDelete} className="text-text-danger">
            <Trash2 className="size-4" />
            Delete conversation
          </ContextMenuItem>
        </>
      )}
    </ContextMenuContent>
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
}: {
  target: ChatRowActionTarget;
  children: ReactNode;
}) {
  return (
    <ContextMenu>
      <ContextMenuTrigger asChild>{children}</ContextMenuTrigger>
      <ChatRowContextMenuContent target={target} />
    </ContextMenu>
  );
}
