import { fireEvent, render, screen } from '@testing-library/react';
import { beforeAll, describe, expect, it, vi } from 'vitest';
import { SidebarProvider, useSidebar } from '../ui/sidebar';
import {
  getSessionTitlePadding,
  getTitlebarControlReserve,
  TitlebarControls,
  TITLEBAR_CONTROL_RESERVE_PROPERTY,
} from './TitlebarControls';

beforeAll(() => {
  Object.defineProperty(window, 'matchMedia', {
    writable: true,
    value: vi.fn().mockImplementation((query: string) => ({
      matches: false,
      media: query,
      onchange: null,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
      addListener: vi.fn(),
      removeListener: vi.fn(),
      dispatchEvent: vi.fn(),
    })),
  });
});

function SidebarState() {
  const { state } = useSidebar();
  return <output data-testid="sidebar-state">{state}</output>;
}

describe('TitlebarControls', () => {
  it('keeps every control in a clickable non-drag layer and toggles the sidebar', () => {
    const onNewWindow = vi.fn();

    render(
      <SidebarProvider defaultOpen={false}>
        <TitlebarControls hidden={false} isMacOS onNewWindow={onNewWindow} />
        <SidebarState />
      </SidebarProvider>
    );

    const controls = screen.getByTestId('titlebar-controls');
    expect(controls).toHaveClass('no-drag', 'pointer-events-auto', 'z-[190]', 'isolate');
    expect(controls).toHaveStyle({ left: '100px' });

    fireEvent.click(screen.getByTestId('titlebar-sidebar-toggle'));
    expect(screen.getByTestId('sidebar-state')).toHaveTextContent('expanded');

    fireEvent.click(screen.getByTestId('titlebar-new-window'));
    expect(onNewWindow).toHaveBeenCalledOnce();
  });

  it('renders exactly the two controls the reserve is sized for', () => {
    render(
      <SidebarProvider defaultOpen={false}>
        <TitlebarControls hidden={false} isMacOS onNewWindow={vi.fn()} />
      </SidebarProvider>
    );

    // Dashboard mode is removed — its titlebar entry point is gone, and
    // the strip width (and therefore the reserve) assumes it stays gone.
    expect(screen.queryByTestId('titlebar-dashboard-toggle')).not.toBeInTheDocument();
    expect(screen.getByTestId('titlebar-sidebar-toggle')).toBeInTheDocument();
    expect(screen.getByTestId('titlebar-new-window')).toBeInTheDocument();
    expect(screen.getByTestId('titlebar-new-window')).toHaveAttribute(
      'title',
      'Start a new chat in a new window'
    );
    // The pair reads alike: both name themselves in the app tooltip, and both hover on the
    // strong fill, the band's fill step (R-27), not a one-off token.
    expect(screen.getByTestId('titlebar-sidebar-toggle')).toHaveAttribute(
      'title',
      'Toggle sidebar'
    );
    expect(screen.getByRole('button', { name: 'Toggle sidebar' })).toBe(
      screen.getByTestId('titlebar-sidebar-toggle')
    );
    for (const id of ['titlebar-sidebar-toggle', 'titlebar-new-window']) {
      expect(screen.getByTestId(id)).toHaveClass('hover:!bg-background-fill-strong');
      expect(screen.getByTestId(id).className).not.toMatch(/background-medium/);
    }
  });

  // A reserve that fails silently is worse than no reserve: the session title
  // and tabs must clear the floating control strip when the sidebar is
  // collapsed, or the macOS traffic lights land on top of them. So assert the
  // actual arithmetic, not merely that some string comes back — a wrong number
  // still returns a perfectly well-formed `var(...)` and looks fine in a test
  // that only checks the shape.
  it('reserves the exact width of the two-control strip', () => {
    // macOS: 100 (traffic lights) + 64 (2 x 32px controls) + 8 (gap) = 172
    expect(getTitlebarControlReserve(true)).toBe(172);
    // Other platforms: 16 (inset) + 64 + 8 = 88
    expect(getTitlebarControlReserve(false)).toBe(88);
  });

  it('feeds the reserve to the session title padding, with a matching fallback', () => {
    expect(TITLEBAR_CONTROL_RESERVE_PROPERTY).toBe('--biorouter-titlebar-control-reserve');

    // The fallback literal must track getTitlebarControlReserve(true); if the
    // strip changes and this is not updated, the padding silently under-reserves
    // on any surface that renders before AppLayout sets the custom property.
    const padding = getSessionTitlePadding(false, true);
    expect(padding).toContain('172px');
    expect(padding).toBe('var(--biorouter-titlebar-control-reserve, 172px)');

    // An open compact sidebar overlay wins over the control reserve.
    expect(getSessionTitlePadding(true, true)).toBe('calc(var(--sidebar-width) + 8px)');
    expect(getSessionTitlePadding(true, false)).toBe('calc(var(--sidebar-width) + 8px)');

    // Nothing to clear -> plain inset.
    expect(getSessionTitlePadding(false, false)).toBe('16px');
  });

  it('does not leave an invisible click layer over an open mobile sidebar', () => {
    render(
      <SidebarProvider>
        <TitlebarControls hidden isMacOS onNewWindow={vi.fn()} />
      </SidebarProvider>
    );

    expect(screen.queryByTestId('titlebar-controls')).not.toBeInTheDocument();
  });
});
