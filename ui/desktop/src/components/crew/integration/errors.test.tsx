import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { composerCopy } from '../composer/copy';
import { CrewHttpError } from '../crewApi';
import { crewObservationCopy } from '../state/copy';
import type { ErrorSource, PaneIntent } from '../state/types';
import { installResizeObserverStub } from '../test/crewTestUtils';
import {
  channelReady,
  currentCrew,
  installDaemon,
  keepEndingWith,
  mocked,
  renderCrew,
  richMessages,
} from './harness';

vi.mock('../crewApi', async () => {
  const actual = await vi.importActual<typeof import('../crewApi')>('../crewApi');
  return { ...actual, crewHttp: vi.fn(), crewRequest: vi.fn(), observeCrew: vi.fn() };
});
// Stable across renders, as the real context's callbacks are.
const config = vi.hoisted(() => ({
  getProviders: async () => [{ name: 'fixture-provider', is_configured: true }],
  read: async () => '',
  getProviderModels: async () => ['fixture-model'],
}));
vi.mock('../../ConfigContext', async () => {
  const actual = await vi.importActual<typeof import('../../ConfigContext')>('../../ConfigContext');
  return { ...actual, useConfig: () => config };
});
vi.mock('../CrewAuthentication', () => ({ default: () => <div /> }));

installResizeObserverStub();

type Region = 'composer' | 'pane' | 'dialog' | 'bar';

function region(name: Region): HTMLElement {
  const element =
    name === 'composer'
      ? document.querySelector<HTMLElement>('.crew-compose')
      : name === 'pane'
        ? document.querySelector<HTMLElement>('aside.crew-pane')
        : name === 'dialog'
          ? screen.getByRole('dialog')
          : screen.getByTestId('crew-connection-bar');
  if (!element) throw new Error(`No ${name} on screen.`);
  return element;
}

interface Case {
  source: ErrorSource;
  pane: PaneIntent;
  expected: Region;
  why: string;
}

/**
 * Every source, each with the details pane open (in the mode that matters) and a dialog open over
 * everything: one slot answers, never two. A source whose surface is not mounted falls back to
 * the connection bar, which every screen mounts.
 */
const CASES: Case[] = [
  { source: 'composer', pane: { mode: 'details' }, expected: 'composer', why: 'a send failure' },
  { source: 'pane:agent', pane: { mode: 'agent' }, expected: 'pane', why: 'a start failure' },
  {
    source: 'pane:agent',
    pane: { mode: 'details' },
    expected: 'bar',
    why: 'a start failure after Ask my agent was replaced',
  },
  {
    source: 'pane:chat-access',
    pane: { mode: 'chat-access', sessionId: 'agent-1' },
    expected: 'pane',
    why: 'a grant failure',
  },
  {
    source: 'pane:details',
    pane: { mode: 'details' },
    expected: 'bar',
    why: 'a details-tab failure (no tab claims its own slot)',
  },
  {
    source: 'dialog:create-team',
    pane: { mode: 'details' },
    expected: 'dialog',
    why: 'the open dialog’s own failure',
  },
  {
    source: 'dialog:invite-people',
    pane: { mode: 'details' },
    expected: 'bar',
    why: 'a failure of a dialog that is no longer open',
  },
  { source: 'observer', pane: { mode: 'details' }, expected: 'bar', why: 'an observer report' },
  { source: 'global', pane: { mode: 'details' }, expected: 'bar', why: 'a global action' },
  {
    source: 'connect',
    pane: { mode: 'details' },
    expected: 'bar',
    why: 'a connect failure with no trust or setup screen on show',
  },
];

describe('every error renders exactly once (ui-redesign-spec, “Where errors render”)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    installDaemon({ messages: richMessages() });
  });

  it.each(CASES)(
    '$source renders once, in the $expected — $why',
    async ({ source, pane, expected }) => {
      renderCrew('/crew?sessionId=agent-1');
      await channelReady();
      act(() => currentCrew().openPane(pane));
      act(() => currentCrew().openDialog({ kind: 'create-team' }));
      await screen.findByRole('dialog');
      expect(document.querySelector('aside.crew-pane')).toHaveAttribute('data-state', 'open');

      const message = `failure from ${source}`;
      act(() => currentCrew().reportError(message, source));

      await waitFor(() => expect(screen.getAllByText(message)).toHaveLength(1));
      expect(within(region(expected)).getByText(message)).toBeInTheDocument();
    }
  );

  it('shows an observation failure once, in the bar, and fails the pane closed', async () => {
    const daemon = installDaemon({ messages: richMessages() });
    renderCrew();
    await channelReady();
    act(() => currentCrew().openPane({ mode: 'details' }));
    // Connection settings is about the connection, not the verified view, so it stays open.
    act(() => currentCrew().openDialog({ kind: 'connection-settings', connectionId: 'conn-1' }));
    await screen.findByRole('dialog');

    // It ends, and ends the same way once observed again quietly (Q2-01).
    const broke = { type: 'error', error: 'observation broke', code: 'temporary' };
    keepEndingWith(broke);
    act(() => daemon.emit(broke));
    // Nothing verified is left while the saved connection is read again (Q2-01)…
    expect(screen.queryByRole('textbox', { name: 'Message #general' })).toBeNull();

    // …and it still calls the connection connected, so the end that came again is said once, in
    // the bar.
    const stopped = crewObservationCopy.updatesStopped('lab');
    await waitFor(() => expect(screen.getAllByText(stopped)).toHaveLength(1));
    expect(within(region('bar')).getByText(stopped)).toBeInTheDocument();
    expect(screen.queryByText(/observation broke/)).toBeNull();
    expect(screen.getByRole('dialog')).toBeInTheDocument();
    // Nothing verified is left to show: the pane's intent is dropped with the channel view.
    expect(currentCrew().ui.pane).toBeNull();
    expect(document.querySelector('aside.crew-pane[data-state="open"]')).toBeNull();
    expect(screen.queryByRole('textbox', { name: 'Message #general' })).toBeNull();
  });
});

/**
 * QA M5 and M1: the composer printed the broker's `code: text` after "Couldn't send." for every
 * refused post, and a message over the broker's 64 KB went to the broker to be refused.
 */
describe('a refused post, in the composer', () => {
  const posts = () =>
    mocked.crewRequest.mock.calls.filter(([, method]) => method === 'message.post');

  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('keeps a message over 64 KB and says so, without sending it', async () => {
    installDaemon({ messages: richMessages() });
    renderCrew();
    const box = await channelReady();
    // 32,769 two-byte characters: under the limit in characters, over it in bytes.
    const long = 'é'.repeat(32_769);
    fireEvent.change(box, { target: { value: long } });
    fireEvent.click(screen.getByRole('button', { name: composerCopy.send }));
    const note = (await screen.findByText(composerCopy.sendErrorLead)).closest(
      '.crew-compose-note'
    ) as HTMLElement;
    expect(note).toHaveTextContent(composerCopy.tooLong);
    expect(posts()).toHaveLength(0);
    expect(box).toHaveValue(long);
  });

  it('says what a refusal means, not the broker’s code', async () => {
    installDaemon({
      messages: richMessages(),
      request: (method) =>
        method === 'message.post'
          ? Promise.reject(
              new CrewHttpError(
                'storage_failed: restart and recover before further mutations',
                400,
                'crew_request_refused',
                undefined,
                'storage_failed'
              )
            )
          : undefined,
    });
    renderCrew();
    const box = await channelReady();
    fireEvent.change(box, { target: { value: 'hello' } });
    fireEvent.click(screen.getByRole('button', { name: composerCopy.send }));
    // The viewer hosts this workspace, so they are the one to restart it.
    expect(await screen.findByText(composerCopy.storageFailedHost)).toBeInTheDocument();
    expect(screen.queryByText(/storage_failed/)).toBeNull();
    expect(box).toHaveValue('hello');
  });
});
