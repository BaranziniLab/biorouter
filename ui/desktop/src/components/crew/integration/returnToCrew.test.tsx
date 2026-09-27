import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { act, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { composerCopy } from '../composer/copy';
import { timelineCopy } from '../timeline/copy';
import { rememberedPaneIntent, rememberedView } from '../state/viewMemory';
import { installResizeObserverStub } from '../test/crewTestUtils';
import {
  channelReady,
  connection,
  currentCrew,
  ids,
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
 * Live QA round 4, Q4-04 (and Q4-05's arrival). Coming back to Crew slid the channel 360 px as a
 * closed pane played its exit, then painted an 18-block skeleton — even for an empty channel, even
 * a minute after leaving — while the You row lost the person's name and a pane left open on
 * Members came back closed. Now the last verified view is kept for the app session (presentation
 * only, SECURITY-SENSITIVE): it is drawn at once, dimmed and inert as a re-verification draws it,
 * until the fresh view replaces it whole; and it is forgotten on every path that clears protected
 * state.
 */

const DAEMON_SENTENCE =
  'Room observation ended. Clear cached room content and refresh authorized access; a stale cursor requires an explicit fresh history selection.';

/** Remembers whether any skeleton ever mounted: the timeline's, or the main area's bones. */
function watchForSkeletons() {
  const seen = { timeline: false, bones: false };
  const check = () => {
    if (document.querySelector('.crew-timeline-skeleton')) seen.timeline = true;
    if (document.querySelector('.crew-frame-bone-message')) seen.bones = true;
  };
  const observer = new MutationObserver(check);
  observer.observe(document.body, { childList: true, subtree: true, attributes: true });
  check();
  return { seen, stop: () => observer.disconnect() };
}

function timelineSlot(): Element | null {
  return document.querySelector('.crew-frame-timeline');
}
function crewApp(): Element {
  return document.querySelector('.crew-app')!;
}

/** The channel's opening page, as the daemon sends it right after the state frame. */
function openingPage(channelId: string = ids.general) {
  const messages = daemon.state.messages.filter((message) => message.channel_id === channelId);
  return {
    type: 'messages',
    channel_id: channelId,
    messages,
    cursor: messages.length ? messages[messages.length - 1].sequence : null,
    reset: true,
    remaining: 0,
  };
}

let daemon: ScriptedDaemon;
let watcher: ReturnType<typeof watchForSkeletons> | null = null;

beforeEach(() => {
  vi.clearAllMocks();
  window.localStorage.clear();
  daemon = installDaemon({ messages: richMessages() });
});
afterEach(() => {
  watcher?.stop();
  watcher = null;
});

/** Open Crew on #general, verified, then leave it. */
async function visitAndLeave() {
  const first = renderCrew();
  await channelReady();
  await screen.findByText('Counts are in.');
  first.unmount();
}

describe('coming back to Crew draws the last verified view, never a skeleton (Q4-04)', () => {
  it('draws the remembered timeline dimmed and inert, then the fresh one whole', async () => {
    await visitAndLeave();
    expect(rememberedView(connection.id)?.messages).toHaveLength(4);

    daemon.state.hold = true;
    const observed = mocked.observeCrew.mock.calls.length;
    renderCrew();
    watcher = watchForSkeletons();

    // At once, from memory: the channel's last messages, dimmed and inert, and the composer's
    // same-height "Verifying access…" bar; the You row keeps the name.
    expect(await screen.findByText('Counts are in.')).toBeInTheDocument();
    expect(timelineSlot()).toHaveAttribute('inert');
    expect(timelineSlot()?.querySelector('.crew-timeline')).toHaveAttribute(
      'data-readonly',
      'true'
    );
    expect(screen.getByTestId('crew-verifying')).toHaveTextContent(composerCopy.verifying);
    expect(currentCrew().snapshot).toBeNull();
    const nav = screen.getByRole('navigation', { name: 'Crew' });
    expect(within(nav).getAllByText(/Alice Chen/).length).toBeGreaterThan(0);

    // The one observation asks for the channel itself: not the workspace first, then the channel.
    await waitFor(() => expect(mocked.observeCrew.mock.calls.length).toBe(observed + 1));
    expect(mocked.observeCrew.mock.calls[observed]![1]).toBe(ids.general);

    // The fresh state arrives before its page: still the remembered view, still verifying.
    act(() => daemon.emitState());
    expect(currentCrew().snapshot).toBeNull();
    expect(currentCrew().status).toBe('checking');
    expect(screen.getByText('Counts are in.')).toBeInTheDocument();
    expect(timelineSlot()).toHaveAttribute('inert');

    // Its page arrives: the fresh view replaces the remembered one whole.
    act(() => daemon.emit(openingPage()));
    await channelReady();
    expect(currentCrew().status).toBe('connected');
    expect(timelineSlot()).not.toHaveAttribute('inert');
    expect(screen.getByText('Counts are in.')).toBeInTheDocument();
    expect(watcher.seen).toEqual({ timeline: false, bones: false });
  });

  it('shows an empty channel’s intro at once, with no skeleton', async () => {
    daemon.state.messages = [];
    await visitAndLeave2();

    daemon.state.hold = true;
    renderCrew();
    watcher = watchForSkeletons();
    expect(
      await screen.findByRole('heading', { name: timelineCopy.introTitle('general') })
    ).toBeInTheDocument();
    expect(timelineSlot()).toHaveAttribute('inert');

    act(() => daemon.release());
    await channelReady();
    expect(
      screen.getByRole('heading', { name: timelineCopy.introTitle('general') })
    ).toBeInTheDocument();
    expect(watcher.seen).toEqual({ timeline: false, bones: false });
  });

  it('gives the remembered view up when the fresh one’s privacy moved', async () => {
    await visitAndLeave();

    daemon.state.hold = true;
    daemon.state.connectionMode = 'public';
    renderCrew();
    expect(await screen.findByText('Counts are in.')).toBeInTheDocument();

    // The first fresh frame says the connection is Public now: nothing kept under Private stays.
    act(() => daemon.emitState());
    await waitFor(() => expect(screen.queryByText('Counts are in.')).toBeNull());
    expect(rememberedView(connection.id)).toBeNull();

    act(() => daemon.emit(openingPage()));
    await channelReady();
  });

  it('keeps nothing across a Disconnect: the next visit starts from nothing', async () => {
    const first = renderCrew();
    await channelReady();
    await screen.findByText('Counts are in.');
    expect(rememberedView(connection.id)).not.toBeNull();
    await act(async () => {
      await currentCrew().disconnect();
    });
    expect(rememberedView(connection.id)).toBeNull();
    first.unmount();

    // Connected again elsewhere (a terminal): the next visit has nothing remembered to draw.
    daemon.state.hold = true;
    renderCrew();
    await waitFor(() => expect(currentCrew().connectionId).toBe(connection.id));
    expect(screen.queryByText('Counts are in.')).toBeNull();
    expect(currentCrew().lastVerified).toBeNull();
    act(() => daemon.release());
    await channelReady();
  });

  it('keeps nothing once the observer is refused', async () => {
    const first = renderCrew();
    await channelReady();
    await screen.findByText('Counts are in.');
    expect(rememberedView(connection.id)).not.toBeNull();
    const refusal = { type: 'error', code: 'forbidden', clear: true, error: DAEMON_SENTENCE };
    keepEndingWith(refusal);
    act(() => daemon.emit(refusal));
    await waitFor(() => expect(currentCrew().refreshError).not.toBeNull());
    expect(rememberedView(connection.id)).toBeNull();
    first.unmount();

    daemon.state.hold = true;
    renderCrew();
    await waitFor(() => expect(currentCrew().connectionId).toBe(connection.id));
    expect(screen.queryByText('Counts are in.')).toBeNull();
    expect(currentCrew().lastVerified).toBeNull();
  });

  it('keeps nothing once the bridge dropped and the daemon says disconnected', async () => {
    const first = renderCrew();
    await channelReady();
    await screen.findByText('Counts are in.');
    expect(rememberedView(connection.id)).not.toBeNull();
    const answer = daemon.state.http;
    daemon.state.http = (path, method, body) =>
      path === '/connections' && method === 'GET'
        ? { connections: [{ ...connection, status: 'disconnected' }] }
        : answer?.(path, method, body);
    act(() =>
      daemon.emit({
        type: 'error',
        code: 'observation_refused',
        clear: true,
        error: DAEMON_SENTENCE,
      })
    );
    await waitFor(() => expect(currentCrew().screen).toBe('offline'));
    expect(rememberedView(connection.id)).toBeNull();
    first.unmount();
  });
});

/** Open Crew on #general, verified with its (empty) page, then leave it. */
async function visitAndLeave2() {
  const first = renderCrew();
  await channelReady();
  await screen.findByRole('heading', { name: timelineCopy.introTitle('general') });
  first.unmount();
}

describe('the details pane on coming back (Q4-04)', () => {
  it('holds a still pane still in the stylesheet, open or closed', () => {
    // jsdom evaluates no CSS: the rules themselves are the contract `data-pane-still` relies on.
    const css = readFileSync(resolve(__dirname, '../crew-app.css'), 'utf8').replace(
      /\/\*[\s\S]*?\*\//g,
      ''
    );
    const still = (selector: string) =>
      new RegExp(
        `${selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}\\s*\\{\\s*animation:\\s*none;\\s*\\}`
      );
    for (const selector of [
      ".crew-app[data-pane-still] .crew-pane[data-state='open']",
      ".crew-app[data-pane-still] .crew-pane[data-state='closed']",
      ".crew-app[data-pane-still] .crew-pane[data-state='open'] > .crew-pane-content",
    ])
      expect(css).toMatch(still(selector));
  });

  it('never animates a pane that mounts closed, and animates one the person opens', async () => {
    renderCrew();
    await channelReady();
    expect(document.querySelector('.crew-pane')).toHaveAttribute('data-state', 'closed');
    expect(crewApp()).toHaveAttribute('data-pane-still');

    act(() => currentCrew().openPane({ mode: 'details', tab: 'members' }));
    expect(document.querySelector('.crew-pane')).toHaveAttribute('data-state', 'open');
    expect(crewApp()).not.toHaveAttribute('data-pane-still');
    act(() => currentCrew().closePane());
    expect(crewApp()).not.toHaveAttribute('data-pane-still');
  });

  it('opens the pane the person left open, on its tab, without animating it', async () => {
    const first = renderCrew();
    await channelReady();
    act(() => currentCrew().openPane({ mode: 'details', tab: 'members' }));
    expect(rememberedPaneIntent(connection.id)).toEqual({ mode: 'details', tab: 'members' });
    first.unmount();

    renderCrew();
    await channelReady();
    expect(currentCrew().ui.pane).toEqual({ mode: 'details', tab: 'members' });
    expect(document.querySelector('.crew-pane')).toHaveAttribute('data-state', 'open');
    expect(crewApp()).toHaveAttribute('data-pane-still');

    // Closed by the person: remembered closed, and the close animates.
    act(() => currentCrew().closePane());
    expect(crewApp()).not.toHaveAttribute('data-pane-still');
    expect(rememberedPaneIntent(connection.id)).toBeNull();
  });
});
