import { useMemo, useState } from 'react';
import { Plus } from '../../icons/app-icons';
import { useCrew } from '../state/CrewControllerContext';
import { sidebarCopy } from './copy';
import { teamSections, useSidebarView } from './sidebarView';
import { rowKeys, TeamSection, visibleTeamRows } from './TeamSection';
import { useCollapsedTeams } from './useCollapsedTeams';
import { useRovingRows } from './useRovingRows';
import './crew-sidebar.css';

/**
 * Every team section, then the quiet "+ Add team" row, as ONE roving-focus list: ↑/↓ move
 * between rows across teams, Tab leaves it. Shown whenever a verified (or last verified) snapshot
 * exists; while it is only the last verified copy the rows still navigate but nothing is
 * actionable.
 */
export function TeamSections({ renameEnabled = false }: { renameEnabled?: boolean }) {
  const crew = useCrew();
  const { snapshot, verified } = useSidebarView(crew);
  const sections = useMemo(() => teamSections(snapshot), [snapshot]);
  const collapsedTeams = useCollapsedTeams(crew.connectionId);
  const [archivedOpen, setArchivedOpen] = useState<ReadonlySet<string>>(() => new Set());
  const actionable = verified;

  const keys = useMemo(() => {
    const list: string[] = [];
    for (const section of sections) {
      const collapsed = collapsedTeams.isCollapsed(section.id);
      const rows = visibleTeamRows(section, {
        collapsed,
        archivedOpen: archivedOpen.has(section.id),
        channelId: crew.channelId,
      });
      list.push(rowKeys.team(section.id));
      rows.channels.forEach((row) => list.push(rowKeys.channel(row.id)));
      if (!collapsed && section.archived.length > 0) list.push(rowKeys.archived(section.id));
      rows.archived.forEach((row) => list.push(rowKeys.channel(row.id)));
      if (!collapsed && actionable) list.push(rowKeys.addChannel(section.id));
    }
    if (actionable) list.push(rowKeys.addTeam);
    return list;
  }, [sections, collapsedTeams, archivedOpen, crew.channelId, actionable]);

  const roving = useRovingRows(keys, crew.channelId ? rowKeys.channel(crew.channelId) : null);

  if (!snapshot) return null;

  const toggleArchived = (teamId: string, open: boolean) =>
    setArchivedOpen((current) => {
      const next = new Set(current);
      if (open) next.add(teamId);
      else next.delete(teamId);
      return next;
    });

  return (
    <div
      ref={roving.containerRef}
      className="crew-sidebar-section"
      data-crew-sidebar-teams=""
      onKeyDown={roving.onKeyDown}
    >
      {sections.map((section) => (
        <TeamSection
          key={section.id}
          section={section}
          collapsed={collapsedTeams.isCollapsed(section.id)}
          onCollapsedChange={(collapsed) => collapsedTeams.setCollapsed(section.id, collapsed)}
          archivedOpen={archivedOpen.has(section.id)}
          onArchivedOpenChange={(open) => toggleArchived(section.id, open)}
          actionable={actionable}
          renameEnabled={renameEnabled}
          tabIndexFor={roving.tabIndexFor}
          onRowFocus={roving.onRowFocus}
        />
      ))}
      <div className="crew-sidebar-list" data-crew-row-item="">
        <button
          type="button"
          className="crew-sidebar-row no-drag"
          data-quiet="true"
          data-crew-row={rowKeys.addTeam}
          tabIndex={roving.tabIndexFor(rowKeys.addTeam)}
          onFocus={() => roving.onRowFocus(rowKeys.addTeam)}
          disabled={!actionable}
          onClick={() => crew.openDialog({ kind: 'create-team' })}
        >
          <Plus className="crew-sidebar-row-icon" aria-hidden="true" />
          <span className="crew-sidebar-row-name">{sidebarCopy.team.add}</span>
        </button>
      </div>
    </div>
  );
}
