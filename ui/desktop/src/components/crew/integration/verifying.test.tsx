import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { channelHeaderCopy } from '../channel/headerCopy';
import { composerCopy } from '../composer/copy';
import { welcomeCopy } from '../onboarding';
import { crewObservationCopy, crewStatusCopy } from '../state/copy';
import { channelAction, installResizeObserverStub } from '../test/crewTestUtils';
import {
  channelReady,
  currentCrew,
  installDaemon,
  keepEndingWith,
  mocked,
  renderCrew,
  richMessages,
  type ScriptedDaemon,
} from './harness';

vi.mock('../crewApi', async () => {
  const actual = await vi.importActual<typeof import('../crewApi')>('../crewApi');
  return { ...actual, crewHttp: vi.fn(), crewRequest: vi.fn(), observeCrew: vi.fn() };
});
// Stable across renders, as the real context's callbacks are.
const config = vi.hoisted(() => ({
  getProviders: async () => [],
  read: async () => '',
  getProviderModels: async () => [],
}));
vi.mock('../../ConfigContext', async () => {
  const actual = await vi.importActual<typeof import('../../ConfigContext')>('../../ConfigContext');
  return { ...actual, useConfig: () => config };
});
vi.mock('../CrewAuthentication', () => ({ default: () => <div /> }));

installResizeObserverStub();

/**
 * Watches every change to the page and remembers whether the first-run welcome ever showed, or the
 * Crew sidebar ever went away — a flash either way is the defect re-verification must not have.
 */
function watchForFlashes() {
  const seen = { welcome: false, noSidebar: false };
  const check = () => {
    if (document.body.textContent?.includes(welcomeCopy.title)) seen.welcome = true;
    if (!document.querySelector('nav[aria-label="Crew"]')) seen.noSidebar = true;
  };
  const observer = new MutationObserver(check);
  observer.observe(document.body, { childList: true, subtree: true, characterData: true });
  check();
  return { seen, stop: () => observer.disconnect() };
}

describe('re-verification never blanks the page (ui-redesign-spec, “Main-area states”)', () => {
  let daemon: ScriptedDaemon;
  let watcher: ReturnType<typeof watchForFlashes> | null = null;

  beforeEach(() => {
    vi.clearAllMocks();
    daemon = installDaemon({ messages: richMessages() });
  });
  afterEach(() => {
    watcher?.stop();
    watcher = null;
  });

  it('draws the sidebar and header from the last verified view, dims the timeline and holds the composer', async () => {
    renderCrew();
    await channelReady();
    await screen.findByText('Counts are in.');
    watcher = watchForFlashes();

    daemon.state.hold = true;
    const before = mocked.observeCrew.mock.calls.length;
    await channelAction('Refresh channel');
    await waitFor(() => expect(mocked.observeCrew.mock.calls.length).toBeGreaterThan(before));
    await waitFor(() => expect(currentCrew().snapshot).toBeNull());
    expect(currentCrew().lastVerified).not.toBeNull();

    // The sidebar and the channel header stay, drawn from the last verified view.
    const nav = screen.getByRole('navigation', { name: 'Crew' });
    expect(within(nav).getByRole('button', { name: /^lab/ })).toBeInTheDocument();
    expect(within(nav).getByText('general')).toBeInTheDocument();
    expect(within(nav).getByText(crewStatusCopy.checking)).toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: channelHeaderCopy.menuName('general') })
    ).toBeInTheDocument();

    // The timeline shows the last messages, dimmed and inert; nothing in it acts.
    expect(screen.getByText('Counts are in.')).toBeInTheDocument();
    const timeline = document.querySelector('.crew-frame-timeline');
    expect(timeline).toHaveAttribute('inert');
    expect(timeline?.querySelector('.crew-timeline')).toHaveAttribute('data-readonly', 'true');

    // The composer is replaced by the same-height bar; its textarea is not mounted (C13).
    expect(screen.queryByRole('textbox', { name: 'Message #general' })).toBeNull();
    expect(screen.getByTestId('crew-verifying')).toHaveTextContent(composerCopy.verifying);

    act(() => daemon.release());
    const composer = await channelReady();
    expect(composer).toBeEnabled();
    expect(screen.queryByTestId('crew-verifying')).toBeNull();
    expect(document.querySelector('.crew-frame-timeline')).not.toHaveAttribute('inert');

    expect(watcher.seen).toEqual({ welcome: false, noSidebar: false });
  });

  it('keeps the draft through re-verification and hands it back with the composer', async () => {
    renderCrew();
    const composer = await channelReady();
    await screen.findByText('Counts are in.');
    watcher = watchForFlashes();
    fireEvent.change(composer, { target: { value: 'draft kept while verifying' } });

    daemon.state.hold = true;
    await channelAction('Refresh channel');
    await waitFor(() =>
      expect(screen.queryByRole('textbox', { name: 'Message #general' })).toBeNull()
    );
    expect(currentCrew().draft.body).toBe('draft kept while verifying');

    act(() => daemon.release());
    expect(await channelReady()).toHaveValue('draft kept while verifying');
    expect(watcher.seen).toEqual({ welcome: false, noSidebar: false });
  });

  it('drops the last verified view when observation fails, rather than showing it stale', async () => {
    renderCrew();
    await channelReady();
    await screen.findByText('Counts are in.');
    // Present before the failure, so its absence below is the failure's doing, not a stale name.
    const menuName = channelHeaderCopy.menuName('general');
    expect(screen.getByRole('button', { name: menuName })).toBeInTheDocument();

    // It ends, and ends the same way once observed again quietly (Q2-01).
    const broke = { type: 'error', error: 'observation broke', code: 'temporary' };
    keepEndingWith(broke);
    act(() => daemon.emit(broke));

    await waitFor(() => expect(currentCrew().lastVerified).toBeNull());
    expect(screen.queryByText('Counts are in.')).toBeNull();
    expect(screen.queryByRole('button', { name: menuName })).toBeNull();
    // In plain words; the daemon's own sentence is never shown. An end without the workspace's
    // answer is said only once the saved connection was read again, is still connected, and a
    // quiet re-observation ended the same way (Q2-01): nothing stale is drawn meanwhile.
    expect(await screen.findByText(crewObservationCopy.updatesStopped('lab'))).toBeInTheDocument();
    expect(currentCrew().lastVerified).toBeNull();
    expect(screen.queryByText(/observation broke/)).toBeNull();
    // Still never the first-run screen: the workspace is known, only its view is withheld.
    expect(screen.queryByText(welcomeCopy.title)).toBeNull();
    expect(screen.getByRole('navigation', { name: 'Crew' })).toBeInTheDocument();
  });
});
