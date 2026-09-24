import { PanelRight } from '../../icons/app-icons';
import { Badge } from '../../ui/badge';
import { Button } from '../../ui/button';
import { Tooltip, TooltipContent, TooltipTrigger } from '../../ui/Tooltip';
import { cn } from '../../../utils';
import { channelName } from '../identity';
import { AgentAccessChip } from './AgentAccessChip';
import { ChannelMenu } from './ChannelMenu';
import { channelCopy } from './copy';
import { MemberStack } from './MemberStack';
import { activeTasksIn, useChannelPresentation } from './presentation';

export interface ChannelHeaderProps {
  /**
   * Who else can post here, from the Access area's grant list. `chats` counts active chat grants
   * whose destination is this channel; `tasks`, when given, replaces the count the header takes
   * from the viewer's own running tasks. Absent means no grant information yet (chats: 0).
   */
  agentAccess?: { chats: number; tasks?: number } | null;
  /** Offer Rename… in the channel menu (the broker advertises `unique_names_v1`). */
  canRename?: boolean;
  /**
   * The id of a hidden `#name` label, so the layout's channel `<section>` can be named "#methods"
   * with `aria-labelledby` (the `<h1>`'s own name is its menu trigger's).
   */
  titleId?: string;
  /** Layout only. */
  className?: string;
}

function ClassificationBadge({ restricted }: { restricted: boolean }) {
  const label = restricted ? channelCopy.restricted : channelCopy.publicSafe;
  const hint = restricted ? channelCopy.restrictedHint : channelCopy.publicSafeHint;
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Badge tone="neutral" className="no-drag">
          {label}
          <span className="sr-only">: {hint}</span>
        </Badge>
      </TooltipTrigger>
      <TooltipContent>{hint}</TooltipContent>
    </Tooltip>
  );
}

/**
 * The channel's 44px band (ui-redesign-spec, "The channel header and channel menu"):
 *
 *     [h1: # methods ▾] [Restricted] [Archived]      ···      [▣ 2 chats] [(A)(B)(C) 5] [⧉]
 *
 * The name is the page's `<h1>` and the channel menu's trigger. Classification is a neutral badge
 * (the padlock means privacy tier only, and privacy has one home: the status row). The details
 * toggle opens or closes the pane on About; the chip and the stack open it on Access and Members.
 *
 * No part of this band declares a drag region, and every control is `no-drag` (issue #74). It
 * draws from the last verified view while a refresh re-verifies, so it never blinks away.
 */
export function ChannelHeader({
  agentAccess,
  canRename = false,
  titleId,
  className,
}: ChannelHeaderProps) {
  const { crew, channel, dir } = useChannelPresentation();
  if (!channel) return null;

  const pane = crew.ui.pane;
  const detailsOpen = pane?.mode === 'details';
  const chats = agentAccess?.chats ?? 0;
  // While a refresh re-verifies, the runs come from the last verified view too, so who can post
  // here never blinks out of the header.
  const runs = crew.snapshot ? crew.runs : (crew.lastVerified?.runs ?? crew.runs);
  const tasks = agentAccess?.tasks ?? activeTasksIn(runs, channel.id);

  return (
    <header
      className={cn(
        'flex h-chrome shrink-0 items-center gap-2 border-b border-border-subtle bg-sidebar pl-2 pr-3',
        className
      )}
    >
      <h1 className="flex min-w-0 items-center text-label">
        <ChannelMenu canRename={canRename} />
      </h1>
      {titleId && (
        <span id={titleId} hidden>
          {channelName(channel)}
        </span>
      )}
      <ClassificationBadge restricted={channel.classification !== 'public_safe'} />
      {channel.archived && (
        <Badge tone="neutral" className="no-drag">
          {channelCopy.archived}
        </Badge>
      )}
      <div className="ml-auto flex shrink-0 items-center gap-1">
        <AgentAccessChip
          chats={chats}
          tasks={tasks}
          onOpen={() => crew.openPane({ mode: 'details', tab: 'access' })}
        />
        <MemberStack
          memberIds={channel.members}
          dir={dir}
          onOpen={() => crew.openPane({ mode: 'details', tab: 'members' })}
        />
        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              type="button"
              variant="ghost"
              shape="round"
              aria-label={channelCopy.details}
              aria-pressed={detailsOpen}
              className="no-drag"
              onClick={() =>
                detailsOpen ? crew.closePane() : crew.openPane({ mode: 'details', tab: 'about' })
              }
            >
              <PanelRight aria-hidden="true" />
            </Button>
          </TooltipTrigger>
          <TooltipContent>{channelCopy.details}</TooltipContent>
        </Tooltip>
      </div>
    </header>
  );
}
