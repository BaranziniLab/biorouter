import { useCallback, useEffect, useRef, useState, type FocusEvent } from 'react';
import { PanelRight } from '../../icons/app-icons';
import { Badge } from '../../ui/badge';
import { Button } from '../../ui/button';
import { Tooltip, TooltipContent, TooltipTrigger } from '../../ui/Tooltip';
import { cn } from '../../../utils';
import { channelName, channelSlug } from '../identity';
import type { CrewController } from '../state/types';
import { AgentAccessChip } from './AgentAccessChip';
import { ChannelMenu } from './ChannelMenu';
import { channelCopy } from './copy';
import { channelHeaderCopy } from './headerCopy';
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

/** How long Refresh channel's "Up to date" stays, once the channel is verified again. */
export const REFRESHED_NOTICE_MS = 2500;

/**
 * The classification, explained where it is shown. It is a real control, so a keyboard user can
 * reach the explanation a mouse user gets on hover: Tab to it and the same tooltip opens, and a
 * screen reader hears it through the visually hidden text. Pressing it opens the details pane on
 * About, where the classification is described in full — as the header's other chips open theirs.
 * The badge renders as the button (`asChild`), so its fill and its focus surface are one box.
 */
function ClassificationBadge({
  restricted,
  onOpen,
  onTriggerFocus,
}: {
  restricted: boolean;
  onOpen(): void;
  /** The header's Tab-only rule: the pane gives focus back here when it closes. */
  onTriggerFocus(event: FocusEvent<HTMLElement>): void;
}) {
  const label = restricted ? channelCopy.restricted : channelCopy.publicSafe;
  // "Restricted" is about which models may read the channel, never about who may join it (Q2-65).
  const hint = restricted ? channelHeaderCopy.restrictedHint : channelCopy.publicSafeHint;
  const nameSuffix = restricted ? channelHeaderCopy.restrictedNameSuffix : `: ${hint}`;
  return (
    <Tooltip>
      <TooltipTrigger asChild onFocus={onTriggerFocus}>
        <Badge tone="neutral" asChild>
          <button
            type="button"
            className="crew-channel-classification no-drag biorouter-focus-surface"
            onClick={onOpen}
          >
            {label}
            <span className="sr-only">{nameSuffix}</span>
          </button>
        </Badge>
      </TooltipTrigger>
      <TooltipContent>{hint}</TooltipContent>
    </Tooltip>
  );
}

/**
 * A tooltip that opens on hover and on a focus the person moved with Tab, never on a focus a
 * program put there. The details pane hands focus back to its toggle when it closes, and Radix
 * opens a trigger's tooltip on any focus, so "Channel details" popped up after the pane closed
 * and stayed. Pass the returned handler as the trigger's `onFocus`: it runs before Radix's own,
 * and a prevented event opens nothing.
 */
function useTooltipOnTabFocusOnly(): (event: FocusEvent<HTMLElement>) => void {
  const tabbing = useRef(false);
  useEffect(() => {
    const down = (event: KeyboardEvent) => {
      if (event.key === 'Tab') tabbing.current = true;
    };
    const settle = () => {
      tabbing.current = false;
    };
    document.addEventListener('keydown', down, true);
    document.addEventListener('keyup', settle, true);
    document.addEventListener('pointerdown', settle, true);
    window.addEventListener('blur', settle);
    return () => {
      document.removeEventListener('keydown', down, true);
      document.removeEventListener('keyup', settle, true);
      document.removeEventListener('pointerdown', settle, true);
      window.removeEventListener('blur', settle);
    };
  }, []);
  return useCallback((event: FocusEvent<HTMLElement>) => {
    if (!tabbing.current) event.preventDefault();
  }, []);
}

/**
 * Refresh channel, and its answer. The controller's `refresh()` resolves once the workspace list
 * is reloaded, before the channel is verified again, and reports a failure in the connection bar
 * rather than by throwing; so "Up to date" waits for the verified view to come back without a
 * refresh error, shows briefly, and goes. A failure shows nothing here: the bar has it.
 */
function useRefreshFeedback(crew: CrewController) {
  const [phase, setPhase] = useState<'idle' | 'refreshing' | 'settling' | 'done'>('idle');
  const { act, refresh: reload } = crew;
  const refresh = useCallback(() => {
    setPhase('refreshing');
    void act('global', 'refresh', () => reload(), { preserveError: true }).then(() =>
      setPhase((current) => (current === 'refreshing' ? 'settling' : current))
    );
  }, [act, reload]);
  const verified = Boolean(crew.snapshot);
  const failed = Boolean(crew.refreshError);
  useEffect(() => {
    if (phase !== 'settling') return;
    if (failed) setPhase('idle');
    else if (verified) setPhase('done');
  }, [phase, verified, failed]);
  useEffect(() => {
    if (phase !== 'done') return;
    const timer = window.setTimeout(() => setPhase('idle'), REFRESHED_NOTICE_MS);
    return () => window.clearTimeout(timer);
  }, [phase]);
  return { refresh, upToDate: phase === 'done' };
}

/**
 * What the channel menu's copy items announce: "Copied", or "Couldn't copy", read once through
 * the header's own quiet region (the item itself shows it too, while the menu stays open).
 */
function useCopyAnnouncement() {
  const [text, setText] = useState({ words: '', count: 0 });
  const timer = useRef<number | null>(null);
  useEffect(
    () => () => {
      if (timer.current !== null) window.clearTimeout(timer.current);
    },
    []
  );
  const announce = useCallback((copied: boolean) => {
    setText((previous) => ({
      words: copied ? channelHeaderCopy.copied : channelHeaderCopy.copyFailed,
      count: previous.count + 1,
    }));
    if (timer.current !== null) window.clearTimeout(timer.current);
    timer.current = window.setTimeout(
      () => setText((previous) => ({ ...previous, words: '' })),
      REFRESHED_NOTICE_MS
    );
  }, []);
  return { text, announce };
}

/**
 * The page title names the channel and the workspace while a channel is open (WCAG 2.4.2), and
 * the title the app had is put back when the channel closes.
 */
function useChannelPageTitle(slug: string | null, workspace: string) {
  useEffect(() => {
    if (!slug) return;
    const previous = document.title;
    document.title = channelHeaderCopy.pageTitle(slug, workspace);
    return () => {
      document.title = previous;
    };
  }, [slug, workspace]);
}

/**
 * The channel's 44px band (ui-redesign-spec, "The channel header and channel menu"):
 *
 *     [h1: # methods ▾] [Restricted] [Archived]      ···      [▣ 2 chats] [(A)(B)(C) 5] [⧉]
 *
 * The name is the page's `<h1>` and the channel menu's trigger. Classification is a neutral badge
 * (the padlock means privacy tier only, and privacy has one home: the status row), a control whose
 * tooltip explains it and which opens About. The details toggle opens or closes the pane on About, and looks
 * pressed while it is open; the chip and the stack open it on Access and Members. While a channel
 * is open the page title names it, and Refresh channel answers "Up to date" here.
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
  const { crew, channel, dir, workspace } = useChannelPresentation();
  const tabFocusOnly = useTooltipOnTabFocusOnly();
  const { refresh, upToDate } = useRefreshFeedback(crew);
  const copyNotice = useCopyAnnouncement();
  useChannelPageTitle(channel ? channelSlug(channel) : null, workspace);
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
        <ChannelMenu canRename={canRename} onRefresh={refresh} onCopied={copyNotice.announce} />
      </h1>
      {titleId && (
        <span id={titleId} hidden>
          {channelName(channel)}
        </span>
      )}
      <ClassificationBadge
        restricted={channel.classification !== 'public_safe'}
        onOpen={() => crew.openPane({ mode: 'details', tab: 'about' })}
        onTriggerFocus={tabFocusOnly}
      />
      {channel.archived && (
        <Badge tone="neutral" className="no-drag">
          {channelCopy.archived}
        </Badge>
      )}
      {/* Always mounted, so the words are announced when they appear. */}
      <span role="status" className="crew-channel-refreshed text-supporting text-text-muted">
        {upToDate ? channelHeaderCopy.upToDate : ''}
      </span>
      <span role="status" className="sr-only" data-crew-copy-notice="">
        {copyNotice.text.words && <span key={copyNotice.text.count}>{copyNotice.text.words}</span>}
      </span>
      <div className="ml-auto flex shrink-0 items-center gap-1">
        <AgentAccessChip
          chats={chats}
          tasks={tasks}
          onOpen={() => crew.openPane({ mode: 'details', tab: 'access' })}
        />
        <MemberStack
          memberIds={channel.members}
          ownerId={channel.owner_id}
          dir={dir}
          onOpen={() => crew.openPane({ mode: 'details', tab: 'members' })}
        />
        <Tooltip>
          <TooltipTrigger asChild onFocus={tabFocusOnly}>
            <Button
              type="button"
              variant="ghost"
              shape="round"
              aria-label={channelCopy.details}
              aria-pressed={detailsOpen}
              className="crew-details-toggle no-drag"
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
