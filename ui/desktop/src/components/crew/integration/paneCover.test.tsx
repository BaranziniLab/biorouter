import { act, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { ErrorSource } from '../state/types';
import { installResizeObserverStub } from '../test/crewTestUtils';
import {
  channelReady,
  connection,
  currentCrew,
  ids,
  installDaemon,
  mocked,
  ownedRun,
  renderCrew,
  richMessages,
} from './harness';
import { stageShape, stageSkeleton } from './stageSkeleton';

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

/**
 * The connection bar stays outside what a covering pane covers (ui-redesign-spec, "Where errors
 * render: exactly once" and "Widths, the pane and the yield ladder").
 *
 * A covering pane hides `.crew-channel-body`. The bar is the slot for every error whose own surface
 * is not on screen, so it must never be inside that body: when it was, those errors rendered once,
 * under the pane, and nobody could see or hear them. jsdom evaluates no container query, so this
 * file holds the structure, and `paneCover.browser.test.ts` measures the same structure — the
 * skeleton this file compares against — in Chromium with the real stylesheets.
 */

const STOP_REFUSED = 'The workspace could not stop the task.';

/** The connection bar, and proof it is in its own row: not in the body a pane covers, not in the pane. */
function uncoveredBar(): HTMLElement {
  const bar = screen.getByTestId('crew-connection-bar');
  expect(bar).toHaveClass('crew-channel-bar');
  expect(bar.parentElement).toHaveClass('crew-channel');
  expect(bar.closest('.crew-channel-body')).toBeNull();
  expect(bar.closest('.crew-pane')).toBeNull();
  return bar;
}

function skeletonStage(): Element {
  const template = document.createElement('template');
  template.innerHTML = stageSkeleton({ note: true, pane: 'open' });
  const stage = template.content.querySelector('.crew-stage');
  if (!stage) throw new Error('The skeleton has no .crew-stage.');
  return stage;
}

const now = () => Math.floor(Date.now() / 1000);

/** A task posting in #general whose run is still going: the Access tab offers Stop on it. */
const taskGrant = () => ({
  session_id: ownedRun.session_id,
  run_id: ownedRun.run_id,
  connection_id: connection.id,
  channel_id: ids.general,
  source_channels: [ids.general],
  policy_epoch: 1,
  expired: false,
  kind: 'task',
  session_name: 'Plot counts',
  expires_at: now() + 3600,
});

describe('the connection bar beside a covering pane', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('renders the stage in the shape the browser measurement loads', async () => {
    installDaemon({ messages: richMessages() });
    renderCrew();
    await channelReady();
    act(() => currentCrew().openPane({ mode: 'details' }));
    act(() => currentCrew().reportError('a global failure', 'global'));
    await screen.findByText('a global failure');

    const stage = document.querySelector('.crew-stage');
    expect(stage).not.toBeNull();
    expect(stageShape(stage!)).toEqual(stageShape(skeletonStage()));
    // And the shape says what it must: the bar is the channel's own row, between band and body.
    expect(stageShape(stage!)).toEqual([
      'section.crew-channel',
      'section.crew-channel > header',
      'section.crew-channel > div.crew-channel-bar.crew-connection-bar',
      'section.crew-channel > div.crew-channel-body',
      'aside.crew-pane',
      'aside.crew-pane > div.crew-pane-content',
    ]);
  });

  it('shows a failed Stop from the pane’s Access tab in the bar, which the pane never covers', async () => {
    installDaemon({
      messages: richMessages(),
      runs: [ownedRun],
      http: (path, method) => {
        if (path === `/connections/${connection.id}/grants` && method === 'GET')
          return { grants: [taskGrant()] };
        if (path === `/connections/${connection.id}/runs/${ownedRun.run_id}/cancel`)
          throw new Error(STOP_REFUSED);
        return undefined;
      },
    });
    renderCrew();
    await channelReady();
    act(() => currentCrew().openPane({ mode: 'details', tab: 'access' }));
    const access = await screen.findByTestId('crew-access-tab');
    const user = userEvent.setup();

    await user.click(await within(access).findByRole('button', { name: 'Stop task' }));
    await user.click(within(access).getByRole('button', { name: 'Stop task' }));

    await waitFor(() => expect(screen.getAllByText(STOP_REFUSED)).toHaveLength(1));
    expect(
      mocked.crewHttp.mock.calls.some(
        ([path, method]) =>
          path === `/connections/${connection.id}/runs/${ownedRun.run_id}/cancel` &&
          method === 'POST'
      )
    ).toBe(true);
    expect(within(uncoveredBar()).getByText(STOP_REFUSED)).toBeInTheDocument();
    // The pane is still open over the channel: the error has to be readable beside it.
    expect(document.querySelector('aside.crew-pane')).toHaveAttribute('data-state', 'open');
  });

  it.each<ErrorSource>(['global', 'observer', 'connect', 'pane:details'])(
    'keeps a %s error outside the covered body with each pane mode open',
    async (source) => {
      installDaemon({ messages: richMessages() });
      renderCrew('/crew?sessionId=agent-1');
      await channelReady();
      for (const pane of [
        { mode: 'details' as const },
        { mode: 'agent' as const },
        { mode: 'chat-access' as const, sessionId: 'agent-1' },
      ]) {
        act(() => currentCrew().openPane(pane));
        const message = `failure from ${source} over ${pane.mode}`;
        act(() => currentCrew().reportError(message, source));
        await waitFor(() => expect(screen.getAllByText(message)).toHaveLength(1));
        expect(within(uncoveredBar()).getByText(message)).toBeInTheDocument();
      }
    }
  );
});
