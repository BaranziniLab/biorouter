import { useId, type KeyboardEvent } from 'react';
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
import { useSidebarCopy } from './SidebarAnnouncer';
import type { ChannelRowView, TeamSectionView } from './sidebarView';
import './crew-sidebar.css';

const copy = sidebarCopy;

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

export interface TeamSectionProps {
  section: TeamSectionView;
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
 */
export function TeamSection({
  section,
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
  const copyText = useSidebarCopy();
  const listId = useId();
  const headerKey = rowKeys.team(section.id);
  const headerTabIndex = tabIndexFor(headerKey);
  const rows = visibleTeamRows(section, { collapsed, archivedOpen, channelId: crew.channelId });
  const channelCount = section.channels.length + section.archived.length;

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
          aria-label={copy.team.toggleLabel(section.name, channelCount)}
          tabIndex={headerTabIndex}
          onFocus={() => onRowFocus(headerKey)}
          onClick={() => onCollapsedChange(!collapsed)}
          onKeyDown={onHeaderKeyDown}
        >
          <ChevronRight className="crew-sidebar-chevron" data-turn="quarter" aria-hidden="true" />
          <bdi className="crew-sidebar-truncate">{section.name}</bdi>
        </button>
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
          <DropdownMenu>
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
              <DropdownMenuItem
                disabled={!actionable}
                onSelect={() =>
                  crew.openDialog({ kind: 'add-people', target: 'team', targetId: section.id })
                }
              >
                {copy.teamMenu.addPeople(section.name)}
              </DropdownMenuItem>
              {renameEnabled && (
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
              <DropdownMenuItem onSelect={() => void copyText(section.id)}>
                {copy.teamMenu.copyId}
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
      </ul>
    </div>
  );
}
