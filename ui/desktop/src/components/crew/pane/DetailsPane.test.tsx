import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { ChannelHeader } from '../channel/ChannelHeader';
import { channelCopy } from '../channel/copy';
import {
  currentCrew,
  general,
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

/** An Access-tab control that switches the pane's mode from inside it. */
function OpenChatAccess() {
  const crew = useCrew();
  return (
    <button type="button" onClick={() => crew.openPane({ mode: 'chat-access' })}>
      Open chat access
    </button>
  );
}

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
          access: (
            <>
              <p>Access slot</p>
              <OpenChatAccess />
            </>
          ),
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
    expect(tabs.map((tab) => tab.textContent)).toEqual([
      'About',
      'Members',
      'Files',
      paneCopy.tabs.access,
    ]);
    // One name for agent access (Q2-66): "Access" alone said nothing about whose.
    expect(paneCopy.tabs.access).toBe('Agent access');
    await waitFor(() => expect(within(pane()).getByRole('tab', { name: 'About' })).toHaveFocus());
    expect(within(pane()).getByRole('heading', { level: 2 })).toHaveTextContent('#general');

    await user.click(within(pane()).getByRole('tab', { name: 'Files' }));
    expect(currentCrew().ui.pane).toEqual({ mode: 'details', tab: 'files' });
    expect(within(pane()).getByText('Files slot')).toBeInTheDocument();
    await user.click(within(pane()).getByRole('tab', { name: paneCopy.tabs.access }));
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

  describe('the × (Q2-68)', () => {
    it('closes the pane on Escape while its tooltip shows', async () => {
      const user = userEvent.setup();
      renderCrew(Layout);
      const toggle = await ready();
      await user.click(toggle);
      await waitFor(() => expect(within(pane()).getByRole('tab', { name: 'About' })).toHaveFocus());
      // Reached by keyboard, the × shows its tooltip, whose own dismissable layer takes Escape
      // first. That is how Escape on "Close details" used to do nothing. It is reached with a real
      // Shift+Tab from the tab list: a focus a program moves shows no tooltip (Q2-56), and without
      // the tooltip this test would pass with the ×'s own Escape handler gone.
      const close = within(pane()).getByRole('button', { name: paneCopy.close });
      await user.tab({ shift: true });
      expect(close).toHaveFocus();
      expect(await screen.findByRole('tooltip')).toHaveTextContent(paneCopy.close);
      await user.keyboard('{Escape}');
      expect(currentCrew().ui.pane).toBeNull();
      await waitFor(() => expect(toggle).toHaveFocus());
    });

    it('is named for what it closes', async () => {
      const user = userEvent.setup();
      renderCrew(Layout);
      await user.click(await ready());
      expect(within(pane()).getByRole('button', { name: paneCopy.close })).toBeInTheDocument();

      await user.click(screen.getByRole('button', { name: 'Ask my agent' }));
      await screen.findByLabelText(agentCopy.task);
      expect(within(pane()).queryByRole('button', { name: paneCopy.close })).toBeNull();
      const close = within(pane()).getByRole('button', { name: paneCopy.closeAgent });
      expect(paneCopy.closeAgent).toBe('Close Ask my agent');

      // Its Escape closes Ask my agent too, and hands focus back to the opener.
      act(() => close.focus());
      await user.keyboard('{Escape}');
      expect(currentCrew().ui.pane).toBeNull();
      await waitFor(() =>
        expect(screen.getByRole('button', { name: 'Ask my agent' })).toHaveFocus()
      );

      act(() => currentCrew().openPane({ mode: 'chat-access' }));
      expect(
        await within(pane()).findByRole('button', { name: paneCopy.closeChatAccess })
      ).toBeInTheDocument();
    });
  });

  it('gives each tab panel a focus indicator for its tab stop (Q2-68)', async () => {
    const user = userEvent.setup();
    renderCrew(Layout);
    await user.click(await ready());
    for (const name of ['About', 'Members', 'Files', paneCopy.tabs.access]) {
      await user.click(within(pane()).getByRole('tab', { name }));
      const panel = within(pane()).getByRole('tabpanel');
      // `main.css` draws the inset edge for `.biorouter-focus-region:focus-visible`; a panel
      // without it was a tab stop that showed nothing.
      expect(panel).toHaveClass('biorouter-focus-region');
      expect(panel).toHaveAttribute('tabindex', '0');
    }
  });

  it('returns focus to the channel menu trigger when the menu opened it', async () => {
    const user = userEvent.setup();
    renderCrew(Layout);
    await ready();
    // The header's own words for its menu belong to the channel area; match the part every
    // version of them has kept.
    const trigger = screen.getByRole('button', { name: /channel menu$/ });
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
    await user.click(within(pane()).getByRole('button', { name: paneCopy.closeChatAccess }));
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

  it('stays open on its tab across a channel switch, where Ask my agent closes, without moving focus (Q2-33)', async () => {
    const user = userEvent.setup();
    renderCrew(Layout);
    await user.click(await ready());
    act(() => currentCrew().openPane({ mode: 'details', tab: 'members' }));
    const composer = screen.getByLabelText('Message #general');
    composer.focus();
    act(() => currentCrew().selectChannel(methods.id));
    await waitFor(() => expect(currentCrew().channel?.id).toBe(methods.id));
    // Details are the channel's facts, so they follow the person to the next channel's.
    expect(currentCrew().ui.pane).toEqual({ mode: 'details', tab: 'members' });
    expect(pane()).toHaveAttribute('data-state', 'open');
    expect(composer).toHaveFocus();

    // Ask my agent is about the channel it was opened on, so a switch closes it.
    await user.click(screen.getByRole('button', { name: 'Ask my agent' }));
    await screen.findByLabelText(agentCopy.task);
    composer.focus();
    act(() => currentCrew().selectChannel(general.id));
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

  // jsdom evaluates no container query, so both titles are in the document here; the classes are
  // what `crew-app.css` shows and hides at the cover threshold.
  it('titles the details mode "Details" while covering, so the header never reads "#general #general" (T-46)', async () => {
    const user = userEvent.setup();
    renderCrew(Layout);
    await user.click(await ready());
    const heading = within(pane()).getByRole('heading', { level: 2 });
    expect(within(heading).getByText('#general')).toHaveClass('crew-push-only');
    expect(within(heading).getByText(paneCopy.coverTitle)).toHaveClass('crew-cover-only');
    expect(paneCopy.coverTitle).toBe('Details');

    // The other modes are titled by what they are, which Back to #name does not repeat.
    await user.click(screen.getByRole('button', { name: 'Ask my agent' }));
    const agentHeading = within(pane()).getByRole('heading', { level: 2 });
    expect(agentHeading).toHaveTextContent(agentCopy.title);
    expect(agentHeading.querySelector('.crew-cover-only, .crew-push-only')).toBeNull();
  });

  it('returns focus to the control that switched its mode, not the one that first opened it', async () => {
    const user = userEvent.setup();
    renderCrew(Layout);
    const toggle = await ready();
    await user.click(toggle);
    await waitFor(() => expect(within(pane()).getByRole('tab', { name: 'About' })).toHaveFocus());
    const ask = screen.getByRole('button', { name: 'Ask my agent' });
    await user.click(ask);
    await waitFor(() => expect(screen.getByLabelText(agentCopy.task)).toHaveFocus());
    await user.keyboard('{Escape}');
    expect(currentCrew().ui.pane).toBeNull();
    await waitFor(() => expect(ask).toHaveFocus());
    expect(toggle).not.toHaveFocus();
  });

  it('keeps the opener when the mode is switched from inside the pane', async () => {
    const user = userEvent.setup();
    renderCrew(Layout);
    const toggle = await ready();
    await user.click(toggle);
    await user.click(within(pane()).getByRole('tab', { name: paneCopy.tabs.access }));
    await user.click(within(pane()).getByRole('button', { name: 'Open chat access' }));
    expect(currentCrew().ui.pane).toEqual({ mode: 'chat-access' });
    await user.click(within(pane()).getByRole('button', { name: paneCopy.closeChatAccess }));
    await waitFor(() => expect(toggle).toHaveFocus());
  });

  describe('an error Ask my agent reported (T-48)', () => {
    it('goes when the pane closes, instead of falling back to the connection bar', async () => {
      const user = userEvent.setup();
      renderCrew(Layout);
      await ready();
      await user.click(screen.getByRole('button', { name: 'Ask my agent' }));
      await screen.findByLabelText(agentCopy.task);
      act(() => currentCrew().reportError('refused here', 'pane:agent'));
      expect(within(pane()).getByRole('alert')).toHaveTextContent('refused here');

      await user.click(within(pane()).getByRole('button', { name: paneCopy.closeAgent }));
      await waitFor(() => expect(currentCrew().error).toBeNull());
      expect(screen.queryByText('refused here')).toBeNull();
    });

    it('goes when the pane switches to another mode', async () => {
      const user = userEvent.setup();
      renderCrew(Layout);
      const toggle = await ready();
      await user.click(screen.getByRole('button', { name: 'Ask my agent' }));
      await screen.findByLabelText(agentCopy.task);
      act(() => currentCrew().reportError('refused here', 'pane:agent'));
      await user.click(toggle);
      await waitFor(() => expect(currentCrew().ui.pane?.mode).toBe('details'));
      await waitFor(() => expect(currentCrew().error).toBeNull());
    });

    it('stays when it arrives after the pane closed, and so is news', async () => {
      const user = userEvent.setup();
      renderCrew(Layout);
      await ready();
      await user.click(screen.getByRole('button', { name: 'Ask my agent' }));
      await screen.findByLabelText(agentCopy.task);
      await user.click(within(pane()).getByRole('button', { name: paneCopy.closeAgent }));
      act(() => currentCrew().reportError('late refusal', 'pane:agent'));
      await waitFor(() => expect(currentCrew().error?.message).toBe('late refusal'));
    });

    it('leaves every other source’s error alone', async () => {
      const user = userEvent.setup();
      renderCrew(Layout);
      await ready();
      await user.click(screen.getByRole('button', { name: 'Ask my agent' }));
      await screen.findByLabelText(agentCopy.task);
      act(() => currentCrew().reportError('connection trouble', 'global'));
      await user.click(within(pane()).getByRole('button', { name: paneCopy.closeAgent }));
      await waitFor(() => expect(currentCrew().ui.pane).toBeNull());
      expect(currentCrew().error?.message).toBe('connection trouble');
    });
  });

  describe('one inset and a ground a step up (T-62)', () => {
    const css = readFileSync(join(__dirname, 'pane.css'), 'utf8').replace(/\/\*[\s\S]*?\*\//g, '');
    const rule = (selector: string) => {
      const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
      return new RegExp(`(^|\\n)${escaped}\\s*\\{([^}]*)\\}`).exec(css)?.[2] ?? '';
    };

    it('declares one 16px inset that the header, body and footer all reach', () => {
      expect(rule('.crew-pane-content')).toMatch(/--crew-pane-inset:\s*16px;/);
      expect(rule('.crew-pane-header')).toMatch(
        /padding-inline:\s*calc\(var\(--crew-pane-inset\) - 8px\);/
      );
      expect(rule('.crew-pane-body')).toMatch(/padding-inline:\s*var\(--crew-pane-inset\);/);
      expect(rule('.crew-pane-footer')).toMatch(/padding-inline:\s*var\(--crew-pane-inset\);/);
    });

    it('paints the pane and its footer on --background-default, not the canvas', () => {
      expect(rule('.crew-pane[data-state]')).toMatch(
        /background-color:\s*var\(--background-default\);/
      );
      expect(rule('.crew-pane-footer')).toMatch(/background-color:\s*var\(--background-default\);/);
      expect(css).not.toMatch(/--background-canvas/);
    });

    it('leaves the inset to the stylesheet rather than a second utility on the body', async () => {
      const user = userEvent.setup();
      renderCrew(Layout);
      await user.click(await ready());
      const bodies = pane().querySelectorAll('.crew-pane-body');
      expect(bodies.length).toBeGreaterThan(0);
      for (const body of bodies) expect(body.className).not.toMatch(/\bpx-\d/);
      expect(within(pane()).getByRole('tablist')).toHaveClass('bg-background-default');
    });
  });
});
