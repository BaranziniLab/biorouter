import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { ChannelHeader } from './ChannelHeader';
import { channelCopy } from './copy';
import {
  currentCrew,
  general,
  installDaemon,
  installObserver,
  makeSnapshot,
  methods,
  renderCrew,
} from './crewTestHarness';

const mocks = vi.hoisted(() => ({
  crewHttp: vi.fn(),
  crewRequest: vi.fn(),
  observeCrew: vi.fn(),
}));

vi.mock('../crewApi', async () => {
  const actual = await vi.importActual<typeof import('../crewApi')>('../crewApi');
  return {
    ...actual,
    crewHttp: mocks.crewHttp,
    crewRequest: mocks.crewRequest,
    observeCrew: mocks.observeCrew,
  };
});

const UUID = /[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/i;

function Header(props: Parameters<typeof ChannelHeader>[0]) {
  function HeaderLayout() {
    return <ChannelHeader {...props} />;
  }
  return HeaderLayout;
}

async function channelShown() {
  return screen.findByRole('button', { name: channelCopy.menuName('general') });
}

beforeEach(() => {
  vi.clearAllMocks();
  installDaemon();
  installObserver();
});

describe('ChannelHeader', () => {
  it('makes the channel name the page heading and the channel menu trigger', async () => {
    renderCrew(Header({ titleId: 'channel-title' }));
    const trigger = await channelShown();
    expect(trigger).toHaveAttribute('aria-haspopup', 'menu');
    expect(trigger).toHaveTextContent('general');
    const heading = screen.getByRole('heading', { level: 1 });
    expect(heading).toContainElement(trigger);
    // The channel section is named "#general" through the hidden label, not the menu's name.
    expect(document.getElementById('channel-title')).toHaveTextContent('#general');
    expect(document.getElementById('channel-title')).not.toBeVisible();
  });

  it('shows the classification as a neutral badge with its consequence, and Archived only when archived', async () => {
    renderCrew(Header({}));
    await channelShown();
    expect(screen.getByText(channelCopy.restricted)).toBeInTheDocument();
    expect(screen.getByText(`: ${channelCopy.restrictedHint}`)).toHaveClass('sr-only');
    expect(screen.queryByText(channelCopy.archived)).toBeNull();
    // The padlock means privacy tier only; classification never draws one.
    expect(screen.queryByTestId('privacy-badge')).toBeNull();
  });

  it('marks an archived channel', async () => {
    installObserver({
      snapshot: makeSnapshot({ channels: [{ ...general, archived: true }, methods] }),
    });
    renderCrew(Header({}));
    // Crew opens the first open channel; an archived one is shown only when chosen.
    await screen.findByRole('button', { name: channelCopy.menuName('methods') });
    expect(screen.queryByText(channelCopy.archived)).toBeNull();
    act(() => currentCrew().selectChannel(general.id));
    await channelShown();
    expect(screen.getByText(channelCopy.archived)).toBeInTheDocument();
  });

  it('counts the channel’s members, not the team’s people, and opens the Members tab', async () => {
    renderCrew(Header({}));
    await channelShown();
    // The team has three people; #general has two.
    const stack = screen.getByRole('button', { name: '2 members' });
    expect(stack).toHaveTextContent('2');
    fireEvent.click(stack);
    expect(currentCrew().ui.pane).toEqual({ mode: 'details', tab: 'members' });
  });

  it('toggles the details pane on About with aria-pressed', async () => {
    renderCrew(Header({}));
    await channelShown();
    const toggle = screen.getByRole('button', { name: channelCopy.details });
    expect(toggle).toHaveAttribute('aria-pressed', 'false');
    fireEvent.click(toggle);
    expect(currentCrew().ui.pane).toEqual({ mode: 'details', tab: 'about' });
    expect(toggle).toHaveAttribute('aria-pressed', 'true');
    fireEvent.click(toggle);
    expect(currentCrew().ui.pane).toBeNull();
    expect(toggle).toHaveAttribute('aria-pressed', 'false');
  });

  it('reads the pressed state as false while the pane shows another mode', async () => {
    renderCrew(Header({}));
    await channelShown();
    act(() => currentCrew().openPane({ mode: 'agent' }));
    const toggle = screen.getByRole('button', { name: channelCopy.details });
    expect(toggle).toHaveAttribute('aria-pressed', 'false');
    fireEvent.click(toggle);
    expect(currentCrew().ui.pane).toEqual({ mode: 'details', tab: 'about' });
  });

  describe('the agent-access chip', () => {
    it('is absent when nothing but people can post', async () => {
      renderCrew(Header({}));
      await channelShown();
      expect(screen.queryByRole('button', { name: /can post here$/ })).toBeNull();
    });

    it('counts the viewer’s running tasks in this channel from the controller', async () => {
      installObserver({
        runs: [
          { run_id: 'run-1', channel_id: general.id, session_id: 's-1', status: 'running' },
          { run_id: 'run-2', channel_id: general.id, session_id: 's-2', status: 'completed' },
          { run_id: 'run-3', channel_id: 'elsewhere', session_id: 's-3', status: 'running' },
        ],
      });
      renderCrew(Header({}));
      await channelShown();
      const chip = await screen.findByRole('button', { name: '1 task can post here' });
      expect(chip).toHaveTextContent('1 task');
    });

    it('labels chats, and chats with tasks as agents, keeping the visible words in the name', async () => {
      const view = renderCrew(Header({ agentAccess: { chats: 2, tasks: 0 } }));
      await channelShown();
      expect(screen.getByRole('button', { name: '2 chats can post here' })).toHaveTextContent(
        '2 chats'
      );
      view.unmount();

      renderCrew(Header({ agentAccess: { chats: 2, tasks: 1 } }));
      await channelShown();
      const chip = screen.getByRole('button', { name: '3 agents can post here' });
      fireEvent.click(chip);
      expect(currentCrew().ui.pane).toEqual({ mode: 'details', tab: 'access' });
    });
  });

  it('keeps every control out of the drag region and never prints an ID', async () => {
    const view = renderCrew(Header({ agentAccess: { chats: 1 } }));
    await channelShown();
    const header = view.container.querySelector('header');
    expect(header).not.toBeNull();
    for (const control of within(header as HTMLElement).getAllByRole('button')) {
      expect(control).toHaveClass('no-drag');
    }
    expect(header?.textContent).not.toMatch(UUID);
  });

  it('stays on screen from the last verified view while a refresh re-verifies', async () => {
    renderCrew(Header({}));
    await channelShown();
    // The next observation never answers, so the snapshot stays cleared.
    mocks.observeCrew.mockImplementation(async () => new Promise(() => undefined));
    await act(async () => {
      void currentCrew().refresh();
    });
    await waitFor(() => expect(currentCrew().snapshot).toBeNull());
    expect(screen.getByRole('button', { name: channelCopy.menuName('general') })).toBeVisible();
  });

  it('renders nothing without a channel', async () => {
    installObserver({ snapshot: makeSnapshot({ channels: [] }) });
    const view = renderCrew(Header({}));
    await waitFor(() => expect(currentCrew().snapshot).not.toBeNull());
    expect(view.container.querySelector('header')).toBeNull();
  });
});
