import { ChevronDown } from '../../icons/app-icons';
import { DropdownMenu, DropdownMenuTrigger } from '../../ui/dropdown-menu';
import { useCrew } from '../state/CrewControllerContext';
import { sidebarCopy } from './copy';
import { useSidebarView } from './sidebarView';
import { WorkspaceMenu } from './WorkspaceMenu';
import './crew-sidebar.css';

/**
 * The switcher band (ui-redesign-spec, "Placement, the 44px band and the drag region"): the
 * `h-chrome` top of the Crew sidebar, holding the workspace name as a real menu — `lab ▾`, whose
 * chevron turns 180° while the menu is open. It replaces the old workspace `<select>`, the
 * connection card, the Reconnect/Authenticate/Edit links and the top bar's "Add workspace".
 *
 * ⚠ **The titlebar reserve is a MARGIN, never padding (issue #74).** When the app sidebar is
 * collapsed — the default at a 1048px window — the floating titlebar controls (and on macOS the
 * traffic lights) sit over this band. Padding would stay inside the trigger's box, so the box
 * would still start under those controls; a margin moves the box itself clear of them. The
 * value is `AppLayout`'s own `--biorouter-titlebar-control-reserve`, never a literal, so it
 * cannot drift from the strip it clears.
 *
 * The reserve is authored CSS keyed on the app sidebar's own `data-state` (`crew-sidebar.css`),
 * not a React read of `useSidebar()`: it then follows the sidebar on the same frame, and the
 * switcher needs no `SidebarProvider` (the regression tests render Crew without one). Nothing
 * here declares `-webkit-app-region: drag`; the trigger carries `no-drag`. jsdom sees neither
 * the rule nor drag rects: verify in the real app with the app sidebar open and collapsed.
 *
 * ⚠ **The reserve WIDENS the column; it never eats the name (T-21).** Inside a fixed 240px
 * column a 172px macOS reserve left the switcher 51px, and the name read "c.". So while the app
 * sidebar is collapsed `crew-app.css` widens `--crew-sidebar-width` by the reserve (less the
 * band's own inset), and the name keeps at least 10ch before it truncates. The full name is in
 * the `title`, for when it does.
 */
export function WorkspaceSwitcher() {
  const crew = useCrew();
  const { title } = useSidebarView(crew);
  const name = crew.connection ? title : '';

  return (
    <div className="crew-sidebar-band" data-crew-band="switcher">
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <button
            type="button"
            className="crew-sidebar-switcher no-drag"
            disabled={!crew.connection}
          >
            <span className="crew-sidebar-switcher-name" title={name || undefined}>
              {name || sidebarCopy.switcher.loading}
            </span>
            <ChevronDown className="crew-sidebar-chevron" data-turn="half" aria-hidden="true" />
          </button>
        </DropdownMenuTrigger>
        {crew.connection && <WorkspaceMenu title={name} />}
      </DropdownMenu>
    </div>
  );
}
