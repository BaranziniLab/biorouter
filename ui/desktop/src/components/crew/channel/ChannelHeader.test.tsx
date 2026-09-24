import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ChannelHeader, REFRESHED_NOTICE_MS } from './ChannelHeader';
import { channelCopy } from './copy';
import { channelHeaderCopy } from './headerCopy';
import { buildPeopleDirectory } from '../identity';
import { currentMembers } from './MemberStack';
import {
  alice,
  bob,
  carol,
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
  return screen.findByRole('button', { name: channelHeaderCopy.menuName('general') });
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
    // A heading jump reads the channel, then what the control is — never "general channel menu".
    expect(heading).toHaveAccessibleName('#general, channel menu');
    expect(trigger).not.toHaveAttribute('aria-label');
    // The channel section is named "#general" through the hidden label, not the menu's name.
    expect(document.getElementById('channel-title')).toHaveTextContent('#general');
    expect(document.getElementById('channel-title')).not.toBeVisible();
  });

  it('shows the classification as a neutral badge with its consequence, and Archived only when archived', async () => {
    renderCrew(Header({}));
    await channelShown();
    expect(screen.getByText(channelCopy.restricted)).toBeInTheDocument();
    expect(screen.getByText(channelHeaderCopy.restrictedNameSuffix)).toHaveClass('sr-only');
    expect(screen.queryByText(channelCopy.archived)).toBeNull();
    // The padlock means privacy tier only; classification never draws one.
    expect(screen.queryByTestId('privacy-badge')).toBeNull();
  });

  it('lets a keyboard reach the classification and read its explanation (T-65)', async () => {
    renderCrew(Header({}));
    const trigger = await channelShown();
    // It says which models may read the channel, and that it limits nobody's membership (Q2-65).
    const badge = screen.getByRole('button', {
      name: 'Restricted: only private models can read it. It doesn’t limit who’s in the channel.',
    });
    expect(`${channelCopy.restricted}${channelHeaderCopy.restrictedNameSuffix}`).toBe(
      'Restricted: only private models can read it. It doesn’t limit who’s in the channel.'
    );
    expect(badge).toHaveClass('no-drag', 'biorouter-focus-surface');
    // The next Tab stop after the channel menu, and its explanation opens on that focus.
    act(() => trigger.focus());
    await userEvent.setup().tab();
    expect(badge).toHaveFocus();
    expect(await screen.findByRole('tooltip')).toHaveTextContent(
      'Only private models can read it. It doesn’t limit who’s in the channel.'
    );
    // Pressing it opens where the classification is described in full.
    fireEvent.click(badge);
    expect(currentCrew().ui.pane).toEqual({ mode: 'details', tab: 'about' });
  });

  it('marks an archived channel', async () => {
    installObserver({
      snapshot: makeSnapshot({ channels: [{ ...general, archived: true }, methods] }),
    });
    renderCrew(Header({}));
    // Crew opens the first open channel; an archived one is shown only when chosen.
    await screen.findByRole('button', { name: channelHeaderCopy.menuName('methods') });
    expect(screen.queryByText(channelCopy.archived)).toBeNull();
    act(() => currentCrew().selectChannel(general.id));
    await channelShown();
    expect(screen.getByText(channelCopy.archived)).toBeInTheDocument();
  });

  it('counts only the people in the channel now, owner first, then you, then by name (Q2-54)', async () => {
    const dave = {
      id: '6f1c2a3b-0000-4000-8000-00000000da7e',
      uid: 1003,
      username: 'dave',
      nickname: 'Dave Kim',
    };
    const gone = {
      id: '6f1c2a3b-0000-4000-8000-00000000901e',
      username: 'erin',
      display_name: 'Erin Park',
    };
    const snapshot = makeSnapshot({
      actor: bob,
      principals: [alice, bob, carol, dave],
      former_principals: [gone],
      channels: [{ ...general, members: [gone.id, dave.id, bob.id, carol.id, alice.id] }, methods],
    });
    installObserver({ snapshot });
    renderCrew(Header({}));
    await channelShown();
    // Erin left: "4 members" is Alice, Bob, Carol and Dave, never a former member.
    expect(screen.getByRole('button', { name: '4 members' })).toHaveTextContent('4');
    const dir = buildPeopleDirectory(snapshot as never, null);
    expect(
      currentMembers(snapshot.channels[0].members as string[], dir, alice.id).map(
        ({ person }) => person?.username
      )
    ).toEqual(['alice', 'bob', 'carol', 'dave']);
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

  it('paints the pressed look while the pane is open (T-46)', async () => {
    renderCrew(Header({}));
    await channelShown();
    const toggle = screen.getByRole('button', { name: channelCopy.details });
    // The rule lives in channel.css; the class and the pressed state are what it keys on.
    expect(toggle).toHaveClass('crew-details-toggle');
    fireEvent.click(toggle);
    expect(toggle).toHaveAttribute('aria-pressed', 'true');
    const { readFileSync } = await import('node:fs');
    const css = readFileSync(`${__dirname}/channel.css`, 'utf8');
    expect(css).toMatch(
      /\.crew-details-toggle\[aria-pressed='true'\]\s*\{\s*background-color: var\(--background-medium\);\s*color: var\(--text-default\);/
    );
  });

  it('opens the toggle’s tooltip for a Tab, never for focus a program hands back (T-46)', async () => {
    renderCrew(Header({}));
    await channelShown();
    const toggle = screen.getByRole('button', { name: channelCopy.details });
    // The pane closing puts focus back on its opener by script.
    act(() => toggle.focus());
    expect(toggle).toHaveFocus();
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(screen.queryByRole('tooltip')).toBeNull();

    // From the control before it, Tab moves focus there: that opens it.
    act(() => screen.getByRole('button', { name: '2 members' }).focus());
    await userEvent.setup().tab();
    expect(toggle).toHaveFocus();
    expect(await screen.findByRole('tooltip')).toHaveTextContent(channelCopy.details);
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

      // A refresh clears the live runs while it re-verifies; the chip keeps the last verified count.
      mocks.observeCrew.mockImplementation(async () => new Promise(() => undefined));
      await act(async () => {
        void currentCrew().refresh();
      });
      await waitFor(() => expect(currentCrew().runs).toEqual([]));
      expect(screen.getByRole('button', { name: '1 task can post here' })).toBeInTheDocument();
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
    expect(
      screen.getByRole('button', { name: channelHeaderCopy.menuName('general') })
    ).toBeVisible();
  });

  describe('the page title (T-60)', () => {
    const original = 'Biorouter - Crew';
    beforeEach(() => {
      document.title = original;
    });
    afterEach(() => {
      document.title = '';
    });

    it('names the channel and the workspace, follows the channel, and is put back on close', async () => {
      const view = renderCrew(Header({}));
      await channelShown();
      await waitFor(() => expect(document.title).toMatch(/^#general · .+ — Biorouter$/));
      act(() => currentCrew().selectChannel(methods.id));
      await screen.findByRole('button', { name: channelHeaderCopy.menuName('methods') });
      await waitFor(() => expect(document.title).toMatch(/^#methods · .+ — Biorouter$/));
      view.unmount();
      expect(document.title).toBe(original);
    });

    it('says the workspace by name, never by ID', () => {
      expect(channelHeaderCopy.pageTitle('general', 'chen-lab')).toBe(
        '#general · chen-lab — Biorouter'
      );
      expect(channelHeaderCopy.pageTitle('general', '')).toBe('#general — Biorouter');
    });
  });

  describe('Refresh channel (T-67)', () => {
    it('answers “Up to date” once the channel is verified again, then lets it go', async () => {
      const user = userEvent.setup();
      renderCrew(Header({}));
      const trigger = await channelShown();
      const status = () =>
        screen
          .getAllByRole('status')
          .find((node) => node.classList.contains('crew-channel-refreshed'));
      expect(status()).toBeEmptyDOMElement();
      await user.click(trigger);
      await user.click(await screen.findByRole('menuitem', { name: channelCopy.menu.refresh }));
      await waitFor(() => expect(status()).toHaveTextContent(channelHeaderCopy.upToDate));
      await waitFor(() => expect(status()).toBeEmptyDOMElement(), {
        timeout: REFRESHED_NOTICE_MS + 2000,
      });
    });

    it('says nothing while the channel is not verified again', async () => {
      const user = userEvent.setup();
      renderCrew(Header({}));
      const trigger = await channelShown();
      // The next observation never answers: the refresh resolves, the view stays unverified.
      mocks.observeCrew.mockImplementation(async () => new Promise(() => undefined));
      await user.click(trigger);
      await user.click(await screen.findByRole('menuitem', { name: channelCopy.menu.refresh }));
      await waitFor(() => expect(currentCrew().snapshot).toBeNull());
      await new Promise((resolve) => setTimeout(resolve, 100));
      expect(screen.queryByText(channelHeaderCopy.upToDate)).toBeNull();
    });
  });

  it('renders nothing without a channel', async () => {
    installObserver({ snapshot: makeSnapshot({ channels: [] }) });
    const view = renderCrew(Header({}));
    await waitFor(() => expect(currentCrew().snapshot).not.toBeNull());
    expect(view.container.querySelector('header')).toBeNull();
  });
});
