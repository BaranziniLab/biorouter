import { useRef, useState } from 'react';
import { ChevronDown } from '../../icons/app-icons';
import { DropdownMenu, DropdownMenuTrigger } from '../../ui/dropdown-menu';
import { Tooltip, TooltipContent, TooltipTrigger } from '../../ui/Tooltip';
import { useCrew } from '../state/CrewControllerContext';
import { sidebarCopy } from './copy';
import { useSidebarView } from './sidebarView';
import { WorkspaceMenu } from './WorkspaceMenu';
import './crew-sidebar.css';

/** Whether an element's text is cut off: it needs more room than it has. */
export function isTruncated(element: HTMLElement | null): boolean {
  return Boolean(element) && element!.scrollWidth > element!.clientWidth;
}

/**
 * The switcher band (ui-redesign-spec, "Placement, the 44px band and the drag region"): the
 * `h-chrome` top of the Crew sidebar, holding the workspace name as a real menu — `lab ▾`, whose
 * chevron turns 180° while the menu is open. It replaces the old workspace `<select>`, the
 * connection card, the Reconnect/Authenticate/Edit links and the top bar's "Add workspace".
 *
 * ⚠ **The titlebar reserve moves the switcher to its own row (issue #74, Q2-39).** When the app
 * sidebar is collapsed — the default at a 1048px window — the floating titlebar controls (and on
 * macOS the traffic lights) sit over this band. The switcher then drops into a 36px row below the
 * band, and the band keeps only the reserve, so nothing of Crew's is under those controls. The
 * column stays 240px: round 1 widened it by the reserve instead (T-21), which left the channel
 * too narrow for the details pane to push. `crew-sidebar.css` keys it on the app sidebar's own
 * `data-state`, not a React read of `useSidebar()`: it follows the sidebar on the same frame, and
 * the switcher needs no `SidebarProvider` (the regression tests render Crew without one). Nothing
 * here declares `-webkit-app-region: drag`; the trigger carries `no-drag`. jsdom sees neither the
 * rule nor drag rects: `crewSidebarGeometry.browser.test.ts` measures it in Chromium.
 *
 * The name keeps at least 10ch before it truncates, and when it does, the full name is in a
 * tooltip that opens to the RIGHT, never down over the status row (Q2-17: it sat over "Updates
 * unavailable"). A name that fits gets no tooltip at all: it would only repeat itself.
 */
export function WorkspaceSwitcher() {
  const crew = useCrew();
  const { title } = useSidebarView(crew);
  const name = crew.connection ? title : '';
  const nameRef = useRef<HTMLSpanElement>(null);
  const [tipOpen, setTipOpen] = useState(false);

  return (
    <div className="crew-sidebar-band" data-crew-band="switcher">
      <DropdownMenu>
        <Tooltip
          open={tipOpen}
          onOpenChange={(next) => setTipOpen(next && Boolean(name) && isTruncated(nameRef.current))}
        >
          {/* The menu's trigger OUTSIDE the tooltip's: its props land last, so the button's
              `data-state` is the menu's, which turns the chevron. */}
          <DropdownMenuTrigger asChild>
            <TooltipTrigger asChild>
              <button
                type="button"
                className="crew-sidebar-switcher no-drag"
                disabled={!crew.connection}
              >
                <span ref={nameRef} className="crew-sidebar-switcher-name">
                  {name || sidebarCopy.switcher.loading}
                </span>
                <ChevronDown className="crew-sidebar-chevron" data-turn="half" aria-hidden="true" />
              </button>
            </TooltipTrigger>
          </DropdownMenuTrigger>
          {name && (
            <TooltipContent side="right" data-crew-switcher-tooltip="">
              {name}
            </TooltipContent>
          )}
        </Tooltip>
        {crew.connection && <WorkspaceMenu title={name} />}
      </DropdownMenu>
    </div>
  );
}
