import { useState } from 'react';
import { ChevronDown } from '../../icons/app-icons';
import { Avatar } from '../../ui/avatar';
import { DropdownMenu, DropdownMenuTrigger } from '../../ui/dropdown-menu';
import { PersonName } from '../identity';
import { useCrew } from '../state/CrewControllerContext';
import { useMenuCopyItem } from './menuCopy';
import { knownUsername, usePendingHost, useSidebarView } from './sidebarView';
import { YouMenu, YouPlace } from './YouMenu';
import './crew-sidebar.css';

/**
 * The dev profile this app runs under, when it runs under one (`BIOROUTER_DEV_PROFILE_NAME`) — in
 * a development build only (Q2-43). A built app launched with a dev profile is what a person
 * meets, and "Profile: frank" means nothing to them.
 */
export function devProfileName(): string | null {
  if (!import.meta.env.DEV) return null;
  try {
    const value = window.appConfig?.get('BIOROUTER_DEV_PROFILE_NAME');
    return typeof value === 'string' && value.trim() ? value.trim() : null;
  } catch {
    return null;
  }
}

/**
 * The You row, the sidebar's pinned 48px footer (ui-redesign-spec, "The Crew sidebar"): who I
 * act as, and where, answered at rest.
 *
 * A 20px avatar, my name (the identity `header` context), and on a second line where I am: "on
 * lab-server", in the sans supporting style (Q4-50). The server is named the person's own way
 * (D-ALIAS): the daemon's `server_label`, their SSH alias for the address, else the host. The line
 * used to be the SSH login in monospace, `crew_carol@lab-server`, which under "Carol Nguyen
 * @crew_carol" said the username a third time.
 *
 * Before a snapshot names me there is no person to show, and then the SSH login is the placeholder
 * (ui-redesign-spec, "The Crew sidebar"): its own monospace text node — the one the regression
 * tests find (`fixture`) — `crew_alice@lab-server` for a saved `crew_alice@52.33.141.141` whose
 * server their SSH configuration calls `lab-server`, else the login exactly as saved, beside an
 * avatar with the username's initial when the login names one (a joiner's blank circle said
 * nothing, Q2-43). The last verified copy of the workspace still names me, so a refresh keeps
 * "on lab-server" rather than flipping to the login and back.
 *
 * A dev profile's "Profile: {name}" badge is in the You MENU's header, not here (T-71): on the
 * row it took about 90px and truncated both lines ("Carol Ng…", "crew_caro…"), and it put a
 * development detail into the row's accessible name.
 *
 * The row is a real menu (`Alice Chen ▾`), so it ends in the same chevron the switcher carries:
 * a dropdown looks like a dropdown. It turns 180° while the menu is open, by the trigger's own
 * `data-state`, through the shared `.crew-sidebar-chevron[data-turn='half']` rule.
 *
 * The menu's open state lives here, with Copy my username's, so a landed copy closes the menu the
 * way every sidebar menu's copy does (Q3-57).
 */
export function YouRow() {
  const crew = useCrew();
  const [open, setOpen] = useState(false);
  const usernameCopy = useMenuCopyItem(setOpen);
  const { dir } = useSidebarView(crew);
  const { username: remembered } = usePendingHost(crew);
  const connection = crew.connection;
  if (!connection) return null;
  const me = dir.me;
  const profile = devProfileName();
  const username = knownUsername(me, connection, remembered);

  return (
    <div className="crew-sidebar-you" data-crew-you="">
      <DropdownMenu open={open} onOpenChange={usernameCopy.onOpenChange}>
        <DropdownMenuTrigger asChild>
          <button type="button" className="crew-sidebar-you-trigger no-drag">
            <Avatar
              size={20}
              fallback={me?.avatar ?? null}
              name={me?.displayName ?? null}
              username={username}
            />
            <span className="crew-sidebar-you-text">
              {me && (
                <span className="crew-sidebar-truncate">
                  <PersonName person={me} context="header" dir={dir} />
                </span>
              )}
              <YouPlace me={me} connection={connection} />
            </span>
            <ChevronDown className="crew-sidebar-chevron" data-turn="half" aria-hidden="true" />
          </button>
        </DropdownMenuTrigger>
        <YouMenu profile={profile} usernameCopy={usernameCopy} />
      </DropdownMenu>
    </div>
  );
}
