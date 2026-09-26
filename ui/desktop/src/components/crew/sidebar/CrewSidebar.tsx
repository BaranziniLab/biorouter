import type { ReactNode } from 'react';
import { useCrew } from '../state/CrewControllerContext';
import { AttentionSections } from './AttentionSections';
import { sidebarCopy } from './copy';
import { SidebarAnnouncer } from './SidebarAnnouncer';
import { usePendingHost } from './sidebarView';
import { StatusRow } from './StatusRow';
import { TeamSections } from './TeamSections';
import { WorkspaceSwitcher } from './WorkspaceSwitcher';
import { YouRow } from './YouRow';
import './crew-sidebar.css';

export interface CrewSidebarProps {
  /**
   * The Agents section (running tasks and connected chats), owned by the access area and passed
   * in so neither area imports the other. It renders nothing when it has no rows.
   */
  agentsSection?: ReactNode;
  /** Offer "Rename team…" once the broker advertises unique names (naming S2). */
  renameEnabled?: boolean;
}

/**
 * The Crew sidebar (ui-redesign-spec, "The Crew sidebar"): `<nav aria-label="Crew">` on the
 * sidebar ground, with a fixed top — the switcher band and the status row — a middle that scrolls
 * on its own, and the pinned You row.
 *
 * The middle holds, each only when it has something: Invitations, Waiting to join and — for the
 * host — Joined, not in your teams (Q3-52), the team sections with "+ Add team", and the Agents
 * slot. The column's 240px width belongs to the
 * layout (`.crew-sidebar` in `crew-app.css`); this component fills it.
 *
 * The team and channel rows are one roving-focus list, so Tab alone reaches only one of them;
 * the landmark's `aria-description` says how the arrow keys move through it, and that Tab reaches
 * a team's own buttons from its header (T-64, Q2-46).
 *
 * A joiner the host has not let in yet has no snapshot, so no sections: the middle then says what
 * will appear there and who it waits for, instead of standing empty (Q2-43).
 *
 * Unmounted until the layout composes it. It reads everything through `useCrew()`, and opens
 * other areas' dialogs and panes through the controller's intents.
 */
export function CrewSidebar({ agentsSection, renameEnabled = false }: CrewSidebarProps) {
  const crew = useCrew();
  const { host } = usePendingHost(crew);
  const pending = crew.status === 'not-joined' && !crew.snapshot;
  return (
    <SidebarAnnouncer>
      <nav
        aria-label={sidebarCopy.navLabel}
        aria-description={sidebarCopy.navDescription}
        className="crew-sidebar-nav"
        aria-busy={crew.connection ? undefined : true}
        data-crew-sidebar-nav=""
      >
        <WorkspaceSwitcher />
        {crew.connection && (
          <>
            <StatusRow />
            <div className="crew-sidebar-scroll" data-crew-sidebar-scroll="">
              {pending && (
                <p className="crew-sidebar-pending text-supporting" data-crew-sidebar-pending="">
                  {sidebarCopy.pendingColumn(host)}
                </p>
              )}
              <AttentionSections />
              <TeamSections renameEnabled={renameEnabled} />
              {agentsSection}
            </div>
            <YouRow />
          </>
        )}
      </nav>
    </SidebarAnnouncer>
  );
}
