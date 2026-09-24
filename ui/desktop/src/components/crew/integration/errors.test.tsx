import { act, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { crewObservationCopy } from '../state/copy';
import type { ErrorSource, PaneIntent } from '../state/types';
import { installResizeObserverStub } from '../test/crewTestUtils';
import { channelReady, currentCrew, installDaemon, renderCrew, richMessages } from './harness';

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

    act(() => daemon.emit({ type: 'error', error: 'observation broke', code: 'temporary' }));

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
