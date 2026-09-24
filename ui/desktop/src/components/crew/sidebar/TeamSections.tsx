import { useMemo, useState } from 'react';
import { Plus } from '../../icons/app-icons';
import type { Snapshot } from '../crewApi';
import { liveInvitations } from '../dialogs/people';
import { personLabel, type PeopleDirectory } from '../identity';
import { useCrew } from '../state/CrewControllerContext';
import { sidebarCopy } from './copy';
import { teamSections, useSidebarView } from './sidebarView';
import { rowKeys, TeamSection, visibleTeamRows, type TeamRole } from './TeamSection';
import { useCollapsedTeams } from './useCollapsedTeams';
import { useRovingRows } from './useRovingRows';
import './crew-sidebar.css';

/**
 * The viewer's standing in each team, by team ID: the OWNER (who created it, and so the only one
 * who can invite people to it) with the people it has invited who have not accepted yet, or a
 * MEMBER.
 *
 * The invitees come from the snapshot's live team invitations (P0-2). "Add @x to Analysis Lab"
 * sends an invitation the invitee must accept, and until they do they are in no list the owner
 * can see — so "Invited" read as "added", and the channel pickers then said nobody was there.
 * The broker shows an inviter every invitation they made; only those still standing count.
 * Display only: the broker decides membership.
 */
export function teamRoles(
  snapshot: Pick<Snapshot, 'actor' | 'teams' | 'invitations'> | null,
  dir: PeopleDirectory,
  nowSeconds?: number
): Map<string, TeamRole> {
  const roles = new Map<string, TeamRole>();
  if (!snapshot || !Array.isArray(snapshot.teams)) return roles;
  const me = snapshot.actor?.id;
  const live = liveInvitations(
    Array.isArray(snapshot.invitations) ? snapshot.invitations : [],
    nowSeconds
  );
  for (const team of snapshot.teams) {
    if (!team || typeof team.id !== 'string') continue;
    if (!me || team.created_by !== me) {
      roles.set(team.id, { kind: 'member' });
      continue;
    }
    // Counted by PERSON, not by invitation: the CLI can mint a second live invitation for someone
    // already invited, and "2 invited" naming one person twice is wrong.
    const invitees = new Set(
      live
        .filter(
          (invitation) =>
            invitation.kind === 'team' &&
            invitation.target_id === team.id &&
            invitation.inviter_id === me &&
            invitation.principal_id !== me
        )
        .map((invitation) => invitation.principal_id)
    );
    const invited = Array.from(invitees, (id) => personLabel(id, 'inline', dir));
    roles.set(team.id, { kind: 'owner', invited });
  }
  return roles;
}

/**
 * Every team section, then the quiet "+ Add team" row, as ONE roving-focus list: ↑/↓ move
 * between rows across teams, Tab leaves it. Shown whenever a verified (or last verified) snapshot
 * exists; while it is only the last verified copy the rows still navigate but nothing is
 * actionable.
 */
export function TeamSections({ renameEnabled = false }: { renameEnabled?: boolean }) {
  const crew = useCrew();
  const { snapshot, verified, dir } = useSidebarView(crew);
  const sections = useMemo(() => teamSections(snapshot), [snapshot]);
  const roles = useMemo(() => teamRoles(snapshot, dir), [snapshot, dir]);
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
          role={roles.get(section.id) ?? { kind: 'member' }}
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
