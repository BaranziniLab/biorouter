import { fireEvent, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { channelAction, installResizeObserverStub } from '../test/crewTestUtils';
import {
  channelReady,
  currentCrew,
  installDaemon,
  mocked,
  renderCrew,
  richMessages,
} from './harness';

vi.mock('../crewApi', async () => {
  const actual = await vi.importActual<typeof import('../crewApi')>('../crewApi');
  return { ...actual, crewHttp: vi.fn(), crewRequest: vi.fn(), observeCrew: vi.fn() };
});
vi.mock('../../ConfigContext', async () => {
  const actual = await vi.importActual<typeof import('../../ConfigContext')>('../../ConfigContext');
  return {
    ...actual,
    useConfig: () => ({
      getProviders: async () => [{ name: 'fixture-provider', is_configured: true }],
      read: async () => '',
      getProviderModels: async () => ['fixture-model'],
    }),
  };
});
vi.mock('../CrewAuthentication', () => ({ default: () => <div /> }));

installResizeObserverStub();

/** The nearest ancestor (or the element) that hides it from assistive technology or input. */
function hiddenBy(element: Element): Element | null {
  for (let node: Element | null = element; node; node = node.parentElement) {
    if (node.getAttribute('aria-hidden') === 'true' || node.hasAttribute('inert')) return node;
  }
  return null;
}

function pane(): HTMLElement {
  const aside = document.querySelector<HTMLElement>('aside.crew-pane');
  if (!aside) throw new Error('No details pane is mounted.');
  return aside;
}

describe('the details pane beside the conversation (ui-redesign-spec, “The details pane”)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    installDaemon({ messages: richMessages() });
  });

  it('leaves the conversation usable while it is open: nothing is hidden, the composer works', async () => {
    renderCrew();
    await channelReady();
    const user = userEvent.setup();
    await user.click(screen.getByRole('button', { name: 'Channel details' }));
    expect(pane()).toHaveAttribute('data-state', 'open');
    expect(screen.getByRole('complementary', { name: /general/ })).toBeInTheDocument();

    const composer = screen.getByRole('textbox', { name: 'Message #general' });
    expect(hiddenBy(composer)).toBeNull();
    expect(hiddenBy(screen.getByRole('button', { name: 'Send message' }))).toBeNull();
    expect(hiddenBy(screen.getByRole('log'))).toBeNull();
    fireEvent.change(composer, { target: { value: 'typed beside the pane' } });
    expect(composer).toHaveValue('typed beside the pane');
    // Non-modal: no scrim, no focus trap, and no dialog semantics.
    expect(screen.queryByRole('dialog')).toBeNull();
  });

  it('closes on Escape from inside and hands focus back to the control that opened it', async () => {
    renderCrew();
    await channelReady();
    const user = userEvent.setup();
    const toggle = screen.getByRole('button', { name: 'Channel details' });
    await user.click(toggle);
    await waitFor(() => expect(pane()).toContainElement(document.activeElement as HTMLElement));

    await user.keyboard('{Escape}');

    await waitFor(() => expect(currentCrew().ui.pane).toBeNull());
    expect(pane()).toHaveAttribute('data-state', 'closed');
    expect(toggle).toHaveFocus();
  });

  it('stays open, with what was typed in it, through a manual refresh', async () => {
    renderCrew();
    await channelReady();
    const user = userEvent.setup();
    await user.click(screen.getByRole('button', { name: 'Ask my agent' }));
    const task = await screen.findByLabelText('Task');
    fireEvent.change(task, { target: { value: 'plot counts by sample' } });

    const before = mocked.observeCrew.mock.calls.length;
    await channelAction('Refresh channel');
    await waitFor(() => expect(mocked.observeCrew.mock.calls.length).toBeGreaterThan(before));
    await channelReady();

    expect(pane()).toHaveAttribute('data-state', 'open');
    expect(currentCrew().ui.pane).toEqual({ mode: 'agent' });
    expect(screen.getByLabelText('Task')).toHaveValue('plot counts by sample');
  });
});
