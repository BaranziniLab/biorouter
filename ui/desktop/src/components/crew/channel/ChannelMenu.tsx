import { useRef, useState } from 'react';
import { ChevronDown, Hash } from '../../icons/app-icons';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '../../ui/dropdown-menu';
import { cn } from '../../../utils';
import { channelName, channelSlug } from '../identity';
import type { DetailsTab, DialogIntent } from '../state/types';
import { channelCopy } from './copy';
import { copyText, useChannelPresentation } from './presentation';
import './channel.css';

export interface ChannelMenuProps {
  /**
   * Offer Rename… to the owner. Only a broker that advertises unique names (`unique_names_v1`,
   * naming slice S2) can rename, and the renderer is not told that yet, so it defaults to off.
   */
  canRename?: boolean;
  /** Layout only, on the trigger. */
  className?: string;
}

/**
 * `# name ▾` — the channel's name is its menu (ui-redesign-spec, "The channel header and channel
 * menu"). The trigger is the page title's content; `ChannelHeader` wraps it in the `<h1>`.
 *
 * Items that open a dialog or the details pane first put focus back on this trigger, so the dialog
 * (or pane) records the trigger as its opener and returns focus to it when it closes; the menu
 * then leaves focus where the new surface put it.
 *
 * Owner items are hidden from everyone else; React gates nothing — the broker refuses anything the
 * person may not do, and the refusal renders in the connection bar.
 */
export function ChannelMenu({ canRename = false, className }: ChannelMenuProps) {
  const { crew, channel, isOwner } = useChannelPresentation();
  const [open, setOpen] = useState(false);
  const trigger = useRef<HTMLButtonElement>(null);
  const handedOff = useRef(false);
  if (!channel) return null;

  const slug = channelSlug(channel);
  const ownerTools = isOwner && !channel.archived;
  const live = crew.historyBefore === null;
  const latest = live ? crew.messages[crew.messages.length - 1] : undefined;

  // Put focus back on the trigger BEFORE the pane or dialog opens, so it records the trigger as
  // its opener and returns focus there when it closes, and keep the menu from moving focus again
  // once it has closed.
  const handOff = (run: () => void) => () => {
    handedOff.current = true;
    trigger.current?.focus();
    run();
  };
  const openDetails = (tab: DetailsTab) => handOff(() => crew.openPane({ mode: 'details', tab }));
  const openDialog = (intent: DialogIntent) => handOff(() => crew.openDialog(intent));

  return (
    <DropdownMenu open={open} onOpenChange={setOpen}>
      <DropdownMenuTrigger asChild>
        <button
          ref={trigger}
          type="button"
          aria-label={channelCopy.menuName(slug)}
          className={cn(
            'crew-channel-title no-drag biorouter-focus-surface flex h-control-md min-w-0 items-center gap-1.5 rounded-element border border-transparent px-2 text-label text-text-default transition-[color,background-color,border-color] hover:border-border-subtle hover:bg-background-medium',
            className
          )}
        >
          <Hash aria-hidden="true" className="h-icon-row w-icon-row shrink-0 text-text-muted" />
          <span className="min-w-0 truncate">{slug}</span>
          <ChevronDown
            aria-hidden="true"
            className="crew-channel-chevron h-icon-row w-icon-row shrink-0 text-text-muted"
          />
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent
        align="start"
        className="w-64"
        onCloseAutoFocus={(event) => {
          if (!handedOff.current) return;
          handedOff.current = false;
          event.preventDefault();
        }}
      >
        <DropdownMenuGroup>
          <DropdownMenuItem onSelect={openDetails('about')}>
            {channelCopy.menu.details}
          </DropdownMenuItem>
          <DropdownMenuItem onSelect={openDetails('members')}>
            {channelCopy.menu.members}
          </DropdownMenuItem>
          <DropdownMenuItem onSelect={openDetails('files')}>
            {channelCopy.menu.files}
          </DropdownMenuItem>
          <DropdownMenuItem onSelect={openDetails('access')}>
            {channelCopy.menu.access}
          </DropdownMenuItem>
        </DropdownMenuGroup>
        <DropdownMenuSeparator />
        <DropdownMenuGroup>
          {ownerTools && (
            <DropdownMenuItem
              onSelect={openDialog({ kind: 'add-people', target: 'channel', targetId: channel.id })}
            >
              {channelCopy.menu.addPeople}
            </DropdownMenuItem>
          )}
          <DropdownMenuItem
            disabled={!latest || crew.isPending('mutate:channel.read')}
            onSelect={() => {
              if (!latest) return;
              // `channel.read` alone: marking a channel read never refreshes (L12).
              void crew.act('global', 'mutate:channel.read', () =>
                crew.markRead(channel.id, latest.sequence)
              );
            }}
          >
            {channelCopy.menu.markRead}
          </DropdownMenuItem>
          <DropdownMenuItem
            onSelect={() =>
              void crew.act('global', 'refresh', () => crew.refresh(), { preserveError: true })
            }
          >
            {channelCopy.menu.refresh}
          </DropdownMenuItem>
          <DropdownMenuItem onSelect={() => void copyText(channelName(channel))}>
            {channelCopy.menu.copyName}
          </DropdownMenuItem>
          <DropdownMenuItem onSelect={() => void copyText(channel.id)}>
            {channelCopy.menu.copyId}
          </DropdownMenuItem>
        </DropdownMenuGroup>
        {ownerTools && (
          <>
            <DropdownMenuSeparator />
            <DropdownMenuGroup>
              {canRename && (
                <DropdownMenuItem
                  onSelect={openDialog({ kind: 'rename', target: 'channel', targetId: channel.id })}
                >
                  {channelCopy.menu.rename}
                </DropdownMenuItem>
              )}
              <DropdownMenuItem
                onSelect={openDialog({ kind: 'transfer-ownership', channelId: channel.id })}
              >
                {channelCopy.menu.transfer}
              </DropdownMenuItem>
              <DropdownMenuItem
                variant="destructive"
                onSelect={openDialog({
                  kind: 'confirm',
                  confirm: { action: 'archive-channel', channelId: channel.id },
                })}
              >
                {channelCopy.menu.archive}
              </DropdownMenuItem>
            </DropdownMenuGroup>
          </>
        )}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
