import { act, fireEvent, render, screen } from '@testing-library/react';
import { MemoryRouter, Route, Routes, useNavigate } from 'react-router-dom';
import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  AppLayout,
  routeOwnsTopBand,
  sidebarAutoCollapseAction,
  TOP_BAND_ROUTE_BODY_CLASS,
} from './AppLayout';

// The layout's own children are not what is under test; each is a heavy surface with its own
// suite. What is under test is the layout's body class, so they are stood in for.
vi.mock('../BioRouterSidebar/AppSidebar', () => ({
  default: () => <div data-testid="app-sidebar" />,
}));
vi.mock('../DependencySetupModal', () => ({ default: () => null }));
vi.mock('../ExtensionUpdateReporter', () => ({ default: () => null }));
vi.mock('../../hooks/useNavigation', () => ({ useNavigation: () => vi.fn() }));

/**
 * The user-reported bug: "the responsiveness of the sidebar collapse button is
 * not working well once the side bar is collapsed (cant bring it back)".
 *
 * Measured in the real app before the fix (Electron, real layout):
 *   1400px, expanded → click toggle → collapsed (railX -240)   ok
 *   shrink to 1000px → still collapsed                          ok
 *   click toggle to reopen → STILL COLLAPSED (railX -240)       BUG
 *
 * Cause: the width watcher had `open` in its deps, so it re-ran as a
 * reconciliation on the user's own toggle and immediately undid it. These tests
 * pin the rule that makes that impossible — auto-collapse fires ONLY on a width
 * crossing. The geometry is verified by driving the app; what is unit-testable
 * here is the decision, which is where the bug actually lived.
 */
describe('sidebarAutoCollapseAction', () => {
  it('does nothing when the width did not cross the threshold', () => {
    // THE REGRESSION. Narrow window, sidebar closed, ref never armed; the user
    // clicks to reopen (open=true) and the watcher re-runs. Before the fix this
    // returned 'collapse' and swallowed the click.
    expect(
      sidebarAutoCollapseAction({
        wasCompact: true,
        isCompact: true,
        open: true,
        autoCollapsed: false,
      })
    ).toBe('none');
  });

  it('never fights a toggle at a stable width, in any ref/open combination', () => {
    for (const compact of [true, false]) {
      for (const open of [true, false]) {
        for (const autoCollapsed of [true, false]) {
          expect(
            sidebarAutoCollapseAction({
              wasCompact: compact,
              isCompact: compact,
              open,
              autoCollapsed,
            })
          ).toBe('none');
        }
      }
    }
  });

  it('collapses when crossing INTO compact while the sidebar is showing', () => {
    expect(
      sidebarAutoCollapseAction({
        wasCompact: false,
        isCompact: true,
        open: true,
        autoCollapsed: false,
      })
    ).toBe('collapse');
  });

  it('collapses on the very first sample in a compact window', () => {
    expect(
      sidebarAutoCollapseAction({
        wasCompact: null,
        isCompact: true,
        open: true,
        autoCollapsed: false,
      })
    ).toBe('collapse');
  });

  it('does not collapse a sidebar the user had already closed', () => {
    expect(
      sidebarAutoCollapseAction({
        wasCompact: false,
        isCompact: true,
        open: false,
        autoCollapsed: false,
      })
    ).toBe('none');
  });

  it('restores on crossing OUT of compact only if it auto-collapsed it', () => {
    expect(
      sidebarAutoCollapseAction({
        wasCompact: true,
        isCompact: false,
        open: false,
        autoCollapsed: true,
      })
    ).toBe('restore');
  });

  it('leaves a hand-closed sidebar closed when the window grows again', () => {
    expect(
      sidebarAutoCollapseAction({
        wasCompact: true,
        isCompact: false,
        open: false,
        autoCollapsed: false,
      })
    ).toBe('none');
  });

  it('replays the exact reported sequence without ever swallowing the toggle', () => {
    // Model the refs the effect keeps, and drive the real sequence.
    let wasCompact: boolean | null = null;
    let autoCollapsed = false;
    let open = true;

    const sample = (isCompact: boolean) => {
      const action = sidebarAutoCollapseAction({ wasCompact, isCompact, open, autoCollapsed });
      wasCompact = isCompact;
      if (action === 'collapse') {
        autoCollapsed = true;
        open = false;
      } else if (action === 'restore') {
        autoCollapsed = false;
        open = true;
      }
      return action;
    };
    // The watcher re-runs after any state change; at a stable width it must
    // always be a no-op.
    const settle = (isCompact: boolean) => sample(isCompact);

    sample(false); // 1. load wide, sidebar open
    expect(open).toBe(true);

    open = false; // 2. user collapses by hand
    expect(settle(false)).toBe('none');
    expect(open).toBe(false);

    expect(sample(true)).toBe('none'); // 3. shrink under the threshold
    expect(open).toBe(false);

    open = true; // 4. user clicks the toggle to bring it back
    expect(settle(true)).toBe('none'); // <-- was 'collapse' before the fix
    expect(open).toBe(true); // the rail STAYS open on the first click
  });
});

const CHAT_ROUTE_CLASS = TOP_BAND_ROUTE_BODY_CLASS;

/** A route body that can move the router, so one mounted layout sees a route change. */
function RouteBody({ path }: { path: string }) {
  const navigate = useNavigate();
  return (
    <div data-testid="route-body" data-path={path}>
      <button onClick={() => navigate('/crew')}>go crew</button>
      <button onClick={() => navigate('/settings')}>go settings</button>
      <button onClick={() => navigate('/welcome')}>go welcome</button>
    </div>
  );
}

function renderLayoutAt(entry: string) {
  return render(
    <MemoryRouter initialEntries={[entry]}>
      <Routes>
        <Route element={<AppLayout />}>
          <Route path="*" element={<RouteBody path={entry} />} />
        </Route>
      </Routes>
    </MemoryRouter>
  );
}

/**
 * The 32px titlebar drag strip stops taking pointer events on a route whose own top band holds
 * controls (issue #74). After the band redesign that is EVERY route inside the shell: the chat
 * header, Crew's bands, and `PageHeader` on every other view, including the not-found page. The
 * routes outside the shell keep the strip. jsdom sees no drag rect; what it can see is the class
 * the `main.css` rule keys on.
 */
describe('the top-band body class', () => {
  afterEach(() => {
    document.body.classList.remove(CHAT_ROUTE_CLASS);
  });

  it('counts every route inside the shell, including the catch-all', () => {
    for (const path of [
      '/',
      '/pair',
      '/crew',
      '/crew/anything',
      '/crewmate',
      '/settings',
      '/extensions',
      '/applications',
      '/sessions',
      '/schedules',
      '/workflows',
      '/skills',
      '/knowledge',
      '/shared-session',
      '/permission',
      '/no-such-page',
    ]) {
      expect(routeOwnsTopBand(path), path).toBe(true);
    }
  });

  it('leaves the routes outside the shell on the drag strip', () => {
    for (const path of [
      '/launcher',
      '/welcome',
      '/configure-providers',
      '/apps',
      '/standalone-app',
    ]) {
      expect(routeOwnsTopBand(path), path).toBe(false);
    }
  });

  it('sets the class on /crew and on a page band route in the real layout', () => {
    const view = renderLayoutAt('/crew');
    expect(screen.getByTestId('route-body')).toBeInTheDocument();
    expect(document.body).toHaveClass(CHAT_ROUTE_CLASS);
    view.unmount();

    renderLayoutAt('/settings');
    expect(document.body).toHaveClass(CHAT_ROUTE_CLASS);
  });

  it('follows the route as it changes, and clears it on unmount', () => {
    const view = renderLayoutAt('/settings');
    expect(document.body).toHaveClass(CHAT_ROUTE_CLASS);

    act(() => {
      fireEvent.click(screen.getByRole('button', { name: 'go welcome' }));
    });
    expect(document.body).not.toHaveClass(CHAT_ROUTE_CLASS);

    act(() => {
      fireEvent.click(screen.getByRole('button', { name: 'go crew' }));
    });
    expect(document.body).toHaveClass(CHAT_ROUTE_CLASS);
    view.unmount();
    expect(document.body).not.toHaveClass(CHAT_ROUTE_CLASS);
  });
});
