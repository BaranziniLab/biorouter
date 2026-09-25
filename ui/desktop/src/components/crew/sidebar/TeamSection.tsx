import { useEffect, useId, useRef, useState, type KeyboardEvent } from 'react';
import { ChevronRight, MoreHorizontal, Plus } from '../../icons/app-icons';
import { Button } from '../../ui/button';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '../../ui/dropdown-menu';
import { Tooltip, TooltipContent, TooltipTrigger } from '../../ui/Tooltip';
import { useCrew } from '../state/CrewControllerContext';
import { ChannelRow } from './ChannelRow';
import { sidebarCopy } from './copy';
import { useSidebarAnnounce, writeClipboard } from './SidebarAnnouncer';
import type { ChannelRowView, TeamSectionView } from './sidebarView';
import './crew-sidebar.css';

const copy = sidebarCopy;

/**
 * How long the team menu stays open showing "Copied" after Copy team ID, before it closes (Q2-34).
 * Long enough to be read, short enough that the menu does not linger.
 */
export const TEAM_COPY_CLOSE_MS = 600;

/** How long a refused copy reads "Couldn't copy" on the item; the menu stays open meanwhile. */
export const TEAM_COPY_FAILED_MS = 1500;

type CopyState = 'idle' | 'copied' | 'failed';

/** Row keys for roving focus. Machine IDs live only in these keys, never on screen. */
export const rowKeys = {
  team: (teamId: string) => `team:${teamId}`,
  channel: (channelId: string) => `channel:${channelId}`,
  archived: (teamId: string) => `archived:${teamId}`,
  addChannel: (teamId: string) => `add-channel:${teamId}`,
  addTeam: 'add-team',
} as const;

/**
 * The rows a team section shows, in order. Collapsed, a section keeps only the selected channel
 * (so the person never loses sight of where they are); expanded, it shows its open channels, the
 * "Archived (n)" toggle and — when that is open — the archived ones.
 */
export function visibleTeamRows(
  section: TeamSectionView,
  opts: { collapsed: boolean; archivedOpen: boolean; channelId: string }
): { channels: ChannelRowView[]; archived: ChannelRowView[] } {
  if (opts.collapsed) {
    return {
      channels: [...section.channels, ...section.archived].filter(
        (row) => row.id === opts.channelId
      ),
      archived: [],
    };
  }
  return { channels: section.channels, archived: opts.archivedOpen ? section.archived : [] };
}

/** The viewer's standing in a team (see `teamRoles`). */
export type TeamRole =
  | {
      kind: 'owner';
      /** People invited who have not accepted yet, as `personLabel`s. */
      invited: string[];
    }
  | { kind: 'member' };

export interface TeamSectionProps {
  section: TeamSectionView;
  /** Whether the viewer owns the team (and whom it invited), or is a member. */
  role: TeamRole;
  /**
   * The viewer may add people to this team: its owner, or — only where the broker adds people
   * directly (`direct_add_v1`) — the workspace's host. The same rule `AddPeopleDialog` states;
   * inviting stays the owner's alone (`invitation.create`), so without direct add a host who did
   * not create the team is not offered it (Q2-41).
   */
  canAddPeople: boolean;
  /**
   * The viewer may rename this team: its owner (creator) only. The broker's `team.rename` gives
   * the host no exception, so offering it to a host who did not create the team led straight to
   * "forbidden: team creator required" (Q2-41).
   */
  canRename: boolean;
  collapsed: boolean;
  onCollapsedChange(collapsed: boolean): void;
  archivedOpen: boolean;
  onArchivedOpenChange(open: boolean): void;
  /** False while the sidebar shows only the last verified copy: nothing is actionable then. */
  actionable: boolean;
  /** The team menu's "Rename team…", once the broker advertises unique names (naming S2). */
  renameEnabled: boolean;
  tabIndexFor(key: string): 0 | -1;
  onRowFocus(key: string): void;
}

/**
 * One team (ui-redesign-spec, "The Crew sidebar"): a header that collapses and expands the
 * section — instantly, only the chevron turns — with a hover- and focus-revealed `+` (Create
 * channel in {team}) and `⋯` (the team menu), then the channel rows, a quiet "Archived (n)"
 * toggle and a quiet "+ Add channel" row.
 *
 * The header is `button[aria-expanded][aria-controls]` named "{team}, {n} channels"; ← collapses
 * and → expands it. The team's name renders as typed, never upper-cased.
 *
 * To the team's owner the header adds "· N invited" while people it invited have not accepted
 * yet, with their names in a tooltip and in the header's description (P0-2). To a member the
 * section ends in a quiet line saying other channels appear once someone adds them (T-28): the
 * snapshot holds only the channels they are in, so nothing else would tell them.
 *
 * The team menu offers "Add people to {team}…" only to those the broker would let add someone —
 * the team's owner, and the host where people are added directly — and "Rename team…" only to the
 * owner (Q2-41). Anyone else would meet a refusal, never an action. "Copy team ID" sits last,
 * after a separator, and answers ON THE ITEM: the menu stays open reading "Copied" for
 * {@link TEAM_COPY_CLOSE_MS}, then closes; a refused copy reads "Couldn't copy" and the menu stays
 * (Q2-34). Either result is spoken too, and neither reaches the
 * channel's connection bar.
 */
export function TeamSection({
  section,
  role,
  canAddPeople,
  canRename,
  collapsed,
  onCollapsedChange,
  archivedOpen,
  onArchivedOpenChange,
  actionable,
  renameEnabled,
  tabIndexFor,
  onRowFocus,
}: TeamSectionProps) {
  const crew = useCrew();
  const { announce } = useSidebarAnnounce();
  const listId = useId();
  const headerKey = rowKeys.team(section.id);
  const headerTabIndex = tabIndexFor(headerKey);
  const invitedId = useId();
  const rows = visibleTeamRows(section, { collapsed, archivedOpen, channelId: crew.channelId });
  const channelCount = section.channels.length + section.archived.length;
  const invited = role.kind === 'owner' ? role.invited : [];

  const onHeaderKeyDown = (event: KeyboardEvent<HTMLButtonElement>) => {
    if (event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return;
    if (event.key === 'ArrowLeft' && !collapsed) {
      event.preventDefault();
      onCollapsedChange(true);
    } else if (event.key === 'ArrowRight' && collapsed) {
      event.preventDefault();
      onCollapsedChange(false);
    }
  };

  const createChannel = () => crew.openDialog({ kind: 'create-channel', teamId: section.id });

  const [menuOpen, setMenuOpen] = useState(false);
  const [copyState, setCopyState] = useState<CopyState>('idle');
  const copyTimer = useRef<number | null>(null);
  const clearCopyTimer = () => {
    if (copyTimer.current !== null) window.clearTimeout(copyTimer.current);
    copyTimer.current = null;
  };
  useEffect(
    () => () => {
      if (copyTimer.current !== null) window.clearTimeout(copyTimer.current);
    },
    []
  );
  const onMenuOpenChange = (open: boolean) => {
    setMenuOpen(open);
    if (!open) {
      clearCopyTimer();
      setCopyState('idle');
    }
  };
  const copyTeamId = async () => {
    const copied = await writeClipboard(section.id);
    clearCopyTimer();
    setCopyState(copied ? 'copied' : 'failed');
    announce(copied ? copy.clipboard.copied : copy.clipboard.failed);
    copyTimer.current = window.setTimeout(
      () => {
        copyTimer.current = null;
        setCopyState('idle');
        if (copied) setMenuOpen(false);
      },
      copied ? TEAM_COPY_CLOSE_MS : TEAM_COPY_FAILED_MS
    );
  };

  return (
    <div className="crew-sidebar-team" data-crew-team={section.id}>
      <div className="crew-sidebar-team-header" data-crew-row-item="">
        <button
          type="button"
          className="crew-sidebar-team-toggle no-drag"
          data-crew-row={headerKey}
          data-crew-team-toggle=""
          aria-expanded={!collapsed}
          aria-controls={listId}
          aria-label={
            invited.length > 0
              ? `${copy.team.toggleLabel(section.name, channelCount)}, ${copy.team.invited(invited.length)}`
              : copy.team.toggleLabel(section.name, channelCount)
          }
          aria-describedby={invited.length > 0 ? invitedId : undefined}
          tabIndex={headerTabIndex}
          onFocus={() => onRowFocus(headerKey)}
          onClick={() => onCollapsedChange(!collapsed)}
          onKeyDown={onHeaderKeyDown}
        >
          <ChevronRight className="crew-sidebar-chevron" data-turn="quarter" aria-hidden="true" />
          <bdi className="crew-sidebar-truncate">{section.name}</bdi>
          {invited.length > 0 && (
            <Tooltip>
              <TooltipTrigger asChild>
                <span className="crew-sidebar-team-invited" data-crew-team-invited="">
                  <span aria-hidden="true">{' · '}</span>
                  {copy.team.invited(invited.length)}
                </span>
              </TooltipTrigger>
              <TooltipContent>{copy.team.invitedNames(invited)}</TooltipContent>
            </Tooltip>
          )}
        </button>
        {invited.length > 0 && (
          <span id={invitedId} className="sr-only">
            {copy.team.invitedNames(invited)}
          </span>
        )}
        <div className="crew-sidebar-team-actions">
          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                variant="ghost"
                size="xs"
                shape="round"
                className="no-drag text-text-muted"
                aria-label={copy.team.addChannel(section.name)}
                tabIndex={headerTabIndex}
                disabled={!actionable}
                onClick={createChannel}
              >
                <Plus aria-hidden="true" />
              </Button>
            </TooltipTrigger>
            <TooltipContent>{copy.team.addChannel(section.name)}</TooltipContent>
          </Tooltip>
          <DropdownMenu open={menuOpen} onOpenChange={onMenuOpenChange}>
            <Tooltip>
              <TooltipTrigger asChild>
                <DropdownMenuTrigger asChild>
                  <Button
                    variant="ghost"
                    size="xs"
                    shape="round"
                    className="no-drag text-text-muted"
                    aria-label={copy.team.options(section.name)}
                    tabIndex={headerTabIndex}
                  >
                    <MoreHorizontal aria-hidden="true" />
                  </Button>
                </DropdownMenuTrigger>
              </TooltipTrigger>
              <TooltipContent>{copy.team.options(section.name)}</TooltipContent>
            </Tooltip>
            <DropdownMenuContent align="start" data-crew-menu="team">
              <DropdownMenuItem disabled={!actionable} onSelect={createChannel}>
                {copy.teamMenu.createChannel}
              </DropdownMenuItem>
              {canAddPeople && (
                <DropdownMenuItem
                  disabled={!actionable}
                  onSelect={() =>
                    crew.openDialog({ kind: 'add-people', target: 'team', targetId: section.id })
                  }
                >
                  {copy.teamMenu.addPeople(section.name)}
                </DropdownMenuItem>
              )}
              {canRename && renameEnabled && (
                <DropdownMenuItem
                  disabled={!actionable}
                  onSelect={() =>
                    crew.openDialog({ kind: 'rename', target: 'team', targetId: section.id })
                  }
                >
                  {copy.teamMenu.rename}
                </DropdownMenuItem>
              )}
              <DropdownMenuSeparator />
              <DropdownMenuItem
                data-crew-copy-state={copyState}
                onSelect={(event) => {
                  // Stay open: the item itself shows whether the copy landed.
                  event.preventDefault();
                  void copyTeamId();
                }}
              >
                {copyState === 'copied'
                  ? copy.teamMenu.copied
                  : copyState === 'failed'
                    ? copy.teamMenu.copyFailed
                    : copy.teamMenu.copyId}
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
        </div>
      </div>
      <ul role="list" id={listId} className="crew-sidebar-list">
        {rows.channels.map((row) => {
          const key = rowKeys.channel(row.id);
          return (
            <ChannelRow
              key={row.id}
              row={row}
              rowKey={key}
              tabIndex={tabIndexFor(key)}
              onRowFocus={onRowFocus}
            />
          );
        })}
        {!collapsed && section.archived.length > 0 && (
          <li data-crew-row-item="">
            <button
              type="button"
              className="crew-sidebar-row no-drag"
              data-quiet="true"
              data-crew-row={rowKeys.archived(section.id)}
              aria-expanded={archivedOpen}
              tabIndex={tabIndexFor(rowKeys.archived(section.id))}
              onFocus={() => onRowFocus(rowKeys.archived(section.id))}
              onClick={() => onArchivedOpenChange(!archivedOpen)}
            >
              <ChevronRight
                className="crew-sidebar-chevron"
                data-turn="quarter"
                aria-hidden="true"
              />
              <span className="crew-sidebar-row-name">
                {copy.channel.archivedGroup(section.archived.length)}
              </span>
            </button>
          </li>
        )}
        {rows.archived.map((row) => {
          const key = rowKeys.channel(row.id);
          return (
            <ChannelRow
              key={row.id}
              row={row}
              rowKey={key}
              tabIndex={tabIndexFor(key)}
              onRowFocus={onRowFocus}
            />
          );
        })}
        {!collapsed && (
          <li data-crew-row-item="">
            <button
              type="button"
              className="crew-sidebar-row no-drag"
              data-quiet="true"
              data-crew-row={rowKeys.addChannel(section.id)}
              tabIndex={tabIndexFor(rowKeys.addChannel(section.id))}
              onFocus={() => onRowFocus(rowKeys.addChannel(section.id))}
              disabled={!actionable}
              onClick={createChannel}
            >
              <Plus className="crew-sidebar-row-icon" aria-hidden="true" />
              <span className="crew-sidebar-row-name">{copy.channel.add}</span>
            </button>
          </li>
        )}
        {!collapsed && role.kind === 'member' && (
          <li className="crew-sidebar-hint text-supporting" data-crew-member-hint="">
            {copy.team.memberHint(section.name)}
          </li>
        )}
      </ul>
    </div>
  );
}
