import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { ChannelHeader } from '../channel/ChannelHeader';
import { channelCopy } from '../channel/copy';
import {
  currentCrew,
  installDaemon,
  installObserver,
  methods,
  renderCrew,
} from '../channel/crewTestHarness';
import { useCrew } from '../state/CrewControllerContext';
import { agentCopy, paneCopy } from './copy';
import { DetailsPane } from './DetailsPane';

const mocks = vi.hoisted(() => ({
  crewHttp: vi.fn(),
  crewRequest: vi.fn(),
  observeCrew: vi.fn(),
  getProviders: vi.fn(),
  read: vi.fn(),
  getProviderModels: vi.fn(),
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
vi.mock('../../ConfigContext', () => ({
  useConfig: () => ({
    getProviders: mocks.getProviders,
    read: mocks.read,
    getProviderModels: mocks.getProviderModels,
  }),
  usePrivacyTiersEnabled: () => true,
}));

function Layout() {
  const crew = useCrew();
  return (
    <div className="crew-stage">
      <section className="crew-channel">
        <ChannelHeader />
        <div className="crew-channel-body">
          <textarea
            aria-label="Message #general"
            value={crew.draft.body}
            onChange={(event) => crew.setBody(event.target.value)}
          />
          <button type="button" onClick={() => crew.openPane({ mode: 'agent' })}>
            Ask my agent
          </button>
        </div>
      </section>
      <DetailsPane
        tabs={{
          files: <p>Files slot</p>,
          access: <p>Access slot</p>,
        }}
        chatAccess={<p>Chat access slot</p>}
      />
    </div>
  );
}

async function ready() {
  await waitFor(() => expect(currentCrew().status).toBe('connected'));
  return screen.findByRole('button', { name: channelCopy.details });
}

function pane() {
  return document.querySelector('aside.crew-pane') as HTMLElement;
}

beforeEach(() => {
  vi.clearAllMocks();
  installDaemon();
  installObserver();
  mocks.getProviders.mockResolvedValue([{ name: 'fixture-provider', is_configured: true }]);
  mocks.getProviderModels.mockResolvedValue(['fixture-model']);
  mocks.read.mockResolvedValue('');
});

describe('DetailsPane', () => {
  it('is a non-modal aside: nothing outside it is hidden and the composer stays usable', async () => {
    const user = userEvent.setup();
    renderCrew(Layout);
    await user.click(await ready());
    const aside = screen.getByRole('complementary', { name: paneCopy.detailsName('#general') });
    expect(aside).toHaveAttribute('data-state', 'open');
    expect(screen.queryByRole('dialog')).toBeNull();

    const composer = screen.getByLabelText('Message #general');
    expect(composer.closest('[aria-hidden="true"]')).toBeNull();
    expect(composer.closest('[inert]')).toBeNull();
    await user.type(composer, 'still typing');
    expect(composer).toHaveValue('still typing');
    // The rest of the page stays reachable: the pane is not a focus trap.
    await user.click(screen.getByRole('button', { name: 'Ask my agent' }));
    expect(currentCrew().ui.pane).toEqual({ mode: 'agent' });
  });

  it('opens on About, focuses the active tab, and switches tabs through the controller', async () => {
    const user = userEvent.setup();
    renderCrew(Layout);
    await user.click(await ready());
    const tabs = within(pane()).getAllByRole('tab');
    expect(tabs.map((tab) => tab.textContent)).toEqual(['About', 'Members', 'Files', 'Access']);
    await waitFor(() => expect(within(pane()).getByRole('tab', { name: 'About' })).toHaveFocus());
    expect(within(pane()).getByRole('heading', { level: 2 })).toHaveTextContent('#general');

    await user.click(within(pane()).getByRole('tab', { name: 'Files' }));
    expect(currentCrew().ui.pane).toEqual({ mode: 'details', tab: 'files' });
    expect(within(pane()).getByText('Files slot')).toBeInTheDocument();
    await user.click(within(pane()).getByRole('tab', { name: 'Access' }));
    expect(within(pane()).getByText('Access slot')).toBeInTheDocument();
  });

  it('closes on Escape from inside and returns focus to the control that opened it', async () => {
    const user = userEvent.setup();
    renderCrew(Layout);
    const toggle = await ready();
    await user.click(toggle);
    await waitFor(() => expect(within(pane()).getByRole('tab', { name: 'About' })).toHaveFocus());
    await user.keyboard('{Escape}');
    expect(currentCrew().ui.pane).toBeNull();
    await waitFor(() => expect(toggle).toHaveFocus());
    expect(pane()).toHaveAttribute('data-state', 'closed');
    expect(pane()).toHaveAttribute('aria-hidden', 'true');
    expect(screen.queryByRole('tab')).toBeNull();
  });

  it('ignores Escape pressed outside it', async () => {
    const user = userEvent.setup();
    renderCrew(Layout);
    await user.click(await ready());
    screen.getByLabelText('Message #general').focus();
    await user.keyboard('{Escape}');
    expect(currentCrew().ui.pane).toEqual({ mode: 'details', tab: 'about' });
  });

  it('returns focus to the channel menu trigger when the menu opened it', async () => {
    const user = userEvent.setup();
    renderCrew(Layout);
    await ready();
    const trigger = screen.getByRole('button', { name: channelCopy.menuName('general') });
    await user.click(trigger);
    await user.click(await screen.findByRole('menuitem', { name: channelCopy.menu.members }));
    expect(currentCrew().ui.pane).toEqual({ mode: 'details', tab: 'members' });
    await waitFor(() => expect(within(pane()).getByRole('tab', { name: 'Members' })).toHaveFocus());
    await user.click(within(pane()).getByRole('button', { name: paneCopy.close }));
    await waitFor(() => expect(trigger).toHaveFocus());
  });

  it('falls back to the composer when the opener is gone', async () => {
    const user = userEvent.setup();
    renderCrew(Layout);
    await ready();
    act(() => currentCrew().openPane({ mode: 'chat-access' }));
    await waitFor(() =>
      expect(within(pane()).getByRole('heading', { name: paneCopy.chatAccessTitle })).toHaveFocus()
    );
    await user.click(within(pane()).getByRole('button', { name: paneCopy.close }));
    await waitFor(() => expect(screen.getByLabelText('Message #general')).toHaveFocus());
  });

  it('switches modes in place, keeping the same aside node', async () => {
    const user = userEvent.setup();
    renderCrew(Layout);
    await user.click(await ready());
    const aside = pane();
    await user.click(screen.getByRole('button', { name: 'Ask my agent' }));
    expect(pane()).toBe(aside);
    expect(screen.getByRole('complementary', { name: agentCopy.title })).toBe(aside);
    await waitFor(() => expect(screen.getByLabelText(agentCopy.task)).toHaveFocus());

    act(() => currentCrew().openPane({ mode: 'chat-access', sessionId: 'chat-1' }));
    expect(pane()).toBe(aside);
    expect(screen.getByRole('complementary', { name: paneCopy.chatAccessTitle })).toBe(aside);
    expect(within(aside).getByText('Chat access slot')).toBeInTheDocument();
  });

  it('survives a refresh, so an error shown in it survives too', async () => {
    const user = userEvent.setup();
    renderCrew(Layout);
    await ready();
    await user.click(screen.getByRole('button', { name: 'Ask my agent' }));
    fireEvent.change(await screen.findByLabelText(agentCopy.task), {
      target: { value: 'typed before the refresh' },
    });
    act(() => currentCrew().reportError('start failed', 'pane:agent'));
    await act(async () => {
      await currentCrew().act('global', 'refresh', () => currentCrew().refresh(), {
        preserveError: true,
      });
    });
    await waitFor(() => expect(currentCrew().status).toBe('connected'));
    expect(currentCrew().ui.pane).toEqual({ mode: 'agent' });
    expect(within(pane()).getAllByText('start failed')).toHaveLength(1);
    expect(screen.getByLabelText(agentCopy.task)).toHaveValue('typed before the refresh');
  });

  it('closes when the channel changes, without moving focus from where the person went', async () => {
    const user = userEvent.setup();
    renderCrew(Layout);
    await user.click(await ready());
    const composer = screen.getByLabelText('Message #general');
    composer.focus();
    act(() => currentCrew().selectChannel(methods.id));
    await waitFor(() => expect(currentCrew().ui.pane).toBeNull());
    expect(composer).toHaveFocus();
  });

  it('offers Back to #name, which closes it (shown only while the pane covers the channel)', async () => {
    const user = userEvent.setup();
    renderCrew(Layout);
    await user.click(await ready());
    const back = within(pane()).getByRole('button', { name: paneCopy.back('#general') });
    expect(back).toHaveClass('crew-cover-only');
    await user.click(back);
    expect(currentCrew().ui.pane).toBeNull();
  });
});
