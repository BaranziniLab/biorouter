import { useCallback, useRef, useState } from 'react';
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
import { CopyForSupport, useMenuCopy } from '../timeline/TimelineCopy';
import { channelCopy } from './copy';
import { channelHeaderCopy } from './headerCopy';
import { copyText, useChannelPresentation } from './presentation';
import './channel.css';

export interface ChannelMenuProps {
  /**
   * Offer Rename… to the owner. Only a broker that advertises unique names (`unique_names_v1`,
   * naming slice S2) can rename, and the renderer is not told that yet, so it defaults to off.
   */
  canRename?: boolean;
  /**
   * Refresh channel. The header passes its own, which answers "Up to date" once the channel is
   * verified again; absent, the item refreshes and says nothing.
   */
  onRefresh?: () => void;
  /**
   * A copy item copied (or the clipboard refused): the header announces it. The item itself
   * reads "Copied" and the menu closes 600ms later (Q2-34).
   */
  onCopied?: (copied: boolean) => void;
  /** Layout only, on the trigger. */
  className?: string;
}

/**
 * `# name ▾` — the channel's name is its menu (ui-redesign-spec, "The channel header and channel
 * menu"). The trigger is the page title's content; `ChannelHeader` wraps it in the `<h1>`. Its
 * name is its content — "#general" and a visually hidden ", channel menu" — and never an
 * `aria-label`, which would replace the heading's own words with the control's.
 *
 * Items that open a dialog or the details pane first put focus back on this trigger, so the dialog
 * (or pane) records the trigger as its opener and returns focus to it when it closes; the menu
 * then leaves focus where the new surface put it.
 *
 * Owner items are hidden from everyone else; React gates nothing — the broker refuses anything the
 * person may not do, and the refusal renders in the connection bar.
 *
 * Copy channel name is a person's copy and stays with the everyday items; Copy channel ID is a
 * machine string, and sits last, after a separator, in "Copy for support" (Q3-26).
 */
export function ChannelMenu({
  canRename = false,
  onRefresh,
  onCopied,
  className,
}: ChannelMenuProps) {
  const { crew, channel, isOwner } = useChannelPresentation();
  const [open, setOpen] = useState(false);
  const trigger = useRef<HTMLButtonElement>(null);
  const handedOff = useRef(false);
  const copied = useRef(onCopied);
  copied.current = onCopied;
  const copy = useCallback(async (text: string) => {
    const landed = await copyText(text);
    copied.current?.(landed);
    return landed;
  }, []);
  const menuCopy = useMenuCopy<'name' | 'id'>(copy, setOpen);
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
    <DropdownMenu open={open} onOpenChange={menuCopy.onOpenChange}>
      <DropdownMenuTrigger asChild>
        <button
          ref={trigger}
          type="button"
          className={cn(
            'crew-channel-title no-drag biorouter-focus-surface flex h-control-md min-w-0 items-center gap-1.5 rounded-element border border-transparent px-2 text-label text-text-default transition-[color,background-color,border-color] hover:border-border-subtle hover:bg-background-medium',
            className
          )}
        >
          <Hash aria-hidden="true" className="h-icon-row w-icon-row shrink-0 text-text-muted" />
          <span className="sr-only">{channelHeaderCopy.hash}</span>
          <span className="min-w-0 truncate">{slug}</span>
          <span className="sr-only">{channelHeaderCopy.menuSuffix}</span>
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
            {channelHeaderCopy.agentAccess}
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
            onSelect={() => {
              if (onRefresh) onRefresh();
              else
                void crew.act('global', 'refresh', () => crew.refresh(), { preserveError: true });
            }}
          >
            {channelCopy.menu.refresh}
          </DropdownMenuItem>
          <DropdownMenuItem
            data-crew-copy-state={menuCopy.state('name')}
            onSelect={menuCopy.select('name', channelName(channel))}
          >
            {menuCopy.label('name', channelCopy.menu.copyName)}
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
        <CopyForSupport label={channelHeaderCopy.copyForSupport}>
          <DropdownMenuItem
            data-crew-copy-state={menuCopy.state('id')}
            onSelect={menuCopy.select('id', channel.id)}
          >
            {menuCopy.label('id', channelCopy.menu.copyId)}
          </DropdownMenuItem>
        </CopyForSupport>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
