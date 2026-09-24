import type { ReactNode } from 'react';
import { useCrew } from '../state/CrewControllerContext';
import { AttentionSections } from './AttentionSections';
import { sidebarCopy } from './copy';
import { SidebarAnnouncer } from './SidebarAnnouncer';
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
 * The middle holds, each only when it has something: Invitations and Waiting to join, the team
 * sections with "+ Add team", and the Agents slot. The column's 240px width belongs to the
 * layout (`.crew-sidebar` in `crew-app.css`); this component fills it.
 *
 * Unmounted until the layout composes it. It reads everything through `useCrew()`, and opens
 * other areas' dialogs and panes through the controller's intents.
 */
export function CrewSidebar({ agentsSection, renameEnabled = false }: CrewSidebarProps) {
  const crew = useCrew();
  return (
    <SidebarAnnouncer>
      <nav
        aria-label={sidebarCopy.navLabel}
        className="crew-sidebar-nav"
        aria-busy={crew.connection ? undefined : true}
        data-crew-sidebar-nav=""
      >
        <WorkspaceSwitcher />
        {crew.connection && (
          <>
            <StatusRow />
            <div className="crew-sidebar-scroll" data-crew-sidebar-scroll="">
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
