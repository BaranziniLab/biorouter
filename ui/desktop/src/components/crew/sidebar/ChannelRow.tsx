import type { KeyboardEvent } from 'react';
import { Hash } from '../../icons/app-icons';
import { Badge } from '../../ui/badge';
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuTrigger,
} from '../../ui/context-menu';
import { useCrew } from '../state/CrewControllerContext';
import { sidebarCopy } from './copy';
import { isContextMenuKey, openContextMenuFromKeyboard } from './keyboardContextMenu';
import { useSidebarCopy } from './SidebarAnnouncer';
import { unreadBadgeText, type ChannelRowView } from './sidebarView';
import './crew-sidebar.css';

/** The pending-action key a sidebar "Mark as read" runs under. */
export const MARK_READ_KEY = 'mark-read';

export interface ChannelRowProps {
  row: ChannelRowView;
  /** The roving-focus key (`channel:<id>`). */
  rowKey: string;
  tabIndex: 0 | -1;
  onRowFocus(key: string): void;
}

/**
 * One channel in a team section (ui-redesign-spec, "The Crew sidebar"): a 32px row with `#` and
 * the name, muted at rest; **unread** is weight and ink plus a neutral count (`99+` cap) — never a
 * hue, and never animated; the selected channel is `aria-current="page"` on the sidebar-active
 * ground with the 2px accent bar.
 *
 * Right-click, Shift+F10 or the Menu key opens its context menu. The two keys are handled HERE,
 * not left to the browser: Chromium sends no `contextmenu` for them on macOS, so without this the
 * menu was pointer-only on the platform this app ships first (see `keyboardContextMenu.ts`). The
 * channel's ID appears only behind "Copy channel ID" (naming rule 8).
 */
export function ChannelRow({ row, rowKey, tabIndex, onRowFocus }: ChannelRowProps) {
  const crew = useCrew();
  const copyText = useSidebarCopy();
  const active = row.id === crew.channelId;
  const unread = row.unread;

  const select = () => {
    if (row.teamId !== crew.teamId) crew.selectTeam(row.teamId);
    crew.selectChannel(row.id);
  };

  const onKeyDown = (event: KeyboardEvent<HTMLButtonElement>) => {
    // Shift+F10 / the Menu key open the row's context menu. Preventing the keydown is what
    // stops Chromium's own dispatch on Linux and Windows, so the menu opens once.
    if (isContextMenuKey(event)) {
      event.preventDefault();
      const row = event.currentTarget;
      openContextMenuFromKeyboard(row, row.querySelector('.crew-sidebar-row-name') ?? row);
      return;
    }
    // ← from a channel returns to its team's header, the way a tree's parent is reached.
    if (event.key !== 'ArrowLeft' || event.altKey || event.ctrlKey || event.metaKey) return;
    const header = event.currentTarget
      .closest('[data-crew-team]')
      ?.querySelector<HTMLElement>('[data-crew-team-toggle]');
    if (!header) return;
    event.preventDefault();
    header.focus();
  };

  const lastMessage = crew.messages[crew.messages.length - 1];
  const canMarkRead =
    active &&
    unread > 0 &&
    crew.messagesLoaded &&
    crew.historyBefore === null &&
    Boolean(lastMessage) &&
    lastMessage.channel_id === row.id;

  return (
    <li data-crew-row-item="">
      <ContextMenu>
        <ContextMenuTrigger asChild>
          <button
            type="button"
            className="crew-sidebar-row no-drag"
            data-crew-row={rowKey}
            data-unread={unread > 0 ? 'true' : undefined}
            data-archived={row.archived ? 'true' : undefined}
            aria-current={active ? 'page' : undefined}
            aria-label={sidebarCopy.channel.rowLabel(row.name, unread)}
            tabIndex={tabIndex}
            onFocus={() => onRowFocus(rowKey)}
            onClick={select}
            onKeyDown={onKeyDown}
          >
            <Hash className="crew-sidebar-row-icon" aria-hidden="true" />
            <span className="crew-sidebar-row-name" translate="no">
              {row.name}
            </span>
            {unread > 0 && (
              <Badge tone="neutral" className="tabular-nums" aria-hidden="true">
                {unreadBadgeText(unread)}
              </Badge>
            )}
          </button>
        </ContextMenuTrigger>
        <ContextMenuContent>
          {canMarkRead && lastMessage && (
            <ContextMenuItem
              onSelect={() =>
                void crew.act('global', MARK_READ_KEY, () =>
                  crew.markRead(row.id, lastMessage.sequence)
                )
              }
            >
              {sidebarCopy.channelMenu.markRead}
            </ContextMenuItem>
          )}
          <ContextMenuItem onSelect={() => void copyText(`#${row.name}`)}>
            {sidebarCopy.channelMenu.copyName}
          </ContextMenuItem>
          <ContextMenuItem onSelect={() => void copyText(row.id)}>
            {sidebarCopy.channelMenu.copyId}
          </ContextMenuItem>
        </ContextMenuContent>
      </ContextMenu>
    </li>
  );
}
