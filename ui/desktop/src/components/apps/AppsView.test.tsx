import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import AppsView from './AppsView';

/**
 * `vi.hoisted`, not a bare `const` closed over by the factory: `vi.mock` calls
 * are hoisted above every declaration in the file, so a factory that referenced
 * a `const` below would throw on initialization rather than mock anything.
 */
const { listApps, useChatContext } = vi.hoisted(() => ({
  listApps: vi.fn(),
  useChatContext: vi.fn(),
}));

vi.mock('../../api', () => ({ listApps }));
vi.mock('../../contexts/ChatContext', () => ({ useChatContext }));

const app = {
  uri: 'ui://spokeagent/cohort',
  name: 'Cohort Browser',
  description: 'Browse a SPOKE cohort.',
  mcpServer: 'spokeagent',
};

const okApps = (apps: unknown[]) => ({ data: { apps } });

describe('AppsView', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    // No chat is open on this route, which is the ordinary case: the view is
    // reached from the sidebar, so `sessionId` is undefined and only the cached
    // listing runs. The error test below installs a session deliberately.
    useChatContext.mockReturnValue(null);
    listApps.mockResolvedValue(okApps([]));
    window.electron = {
      launchApp: vi.fn().mockResolvedValue(undefined),
    } as unknown as typeof window.electron;
  });

  it('renders the shared page header with its title and description', async () => {
    render(<AppsView />);

    expect(await screen.findByRole('heading', { level: 1, name: 'MCP apps' })).toBeInTheDocument();
    expect(
      screen.getByText(
        'Apps your installed extensions provide, which can run in standalone windows. Apps you built yourself live under Built apps.'
      )
    ).toBeInTheDocument();
  });

  it('shows the shared empty state when no extension advertises an app', async () => {
    render(<AppsView />);

    expect(
      await screen.findByRole('heading', { level: 2, name: 'No apps available' })
    ).toBeInTheDocument();
    expect(
      screen.getByText('Install MCP servers that provide UI resources to see apps here.')
    ).toBeInTheDocument();
  });

  it('renders one card per app, each with its server chip and a launch control', async () => {
    listApps.mockResolvedValue(
      okApps([
        app,
        { uri: 'ui://spokeagent/pathway', name: 'Pathway Viewer', mcpServer: 'spokeagent' },
      ])
    );

    render(<AppsView />);

    expect(
      await screen.findByRole('heading', { level: 3, name: 'Cohort Browser' })
    ).toBeInTheDocument();
    expect(screen.getByRole('heading', { level: 3, name: 'Pathway Viewer' })).toBeInTheDocument();
    expect(screen.getByText('Browse a SPOKE cohort.')).toBeInTheDocument();
    expect(screen.getAllByText('spokeagent')).toHaveLength(2);
    expect(screen.getAllByRole('button', { name: 'Launch' })).toHaveLength(2);
    // The empty state and the grid are mutually exclusive.
    expect(screen.queryByRole('heading', { name: 'No apps available' })).not.toBeInTheDocument();
  });

  it('launches the app whose card the button belongs to', async () => {
    listApps.mockResolvedValue(okApps([app]));
    const user = userEvent.setup();

    render(<AppsView />);
    await user.click(await screen.findByRole('button', { name: 'Launch' }));

    expect(window.electron.launchApp).toHaveBeenCalledWith(
      expect.objectContaining({ name: 'Cohort Browser' })
    );
  });

  /**
   * The failure used to `return` a bare centred block ABOVE the header, so a
   * failed load took the page's own title with it. Asserting the heading is
   * still present is the half of this that pins the regression; asserting the
   * note is the half that pins rule 4.
   */
  it('keeps the page header when the load fails and offers Retry inside the note', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    useChatContext.mockReturnValue({ chat: { sessionId: 'session-1' } });
    listApps.mockRejectedValue(new Error('daemon unreachable'));
    const user = userEvent.setup();

    render(<AppsView />);

    const note = await screen.findByRole('alert');
    expect(note).toHaveTextContent('Error loading apps: daemon unreachable');
    expect(screen.getByRole('heading', { level: 1, name: 'MCP apps' })).toBeInTheDocument();
    expect(screen.queryByRole('heading', { name: 'No apps available' })).not.toBeInTheDocument();

    listApps.mockResolvedValue(okApps([app]));
    await user.click(screen.getByRole('button', { name: 'Retry' }));

    expect(
      await screen.findByRole('heading', { level: 3, name: 'Cohort Browser' })
    ).toBeInTheDocument();
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();

    warn.mockRestore();
  });
});
