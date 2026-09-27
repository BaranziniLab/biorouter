import { fireEvent, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { chooseModel, installResizeObserverStub } from '../test/crewTestUtils';
import {
  channelReady,
  connection,
  installDaemon,
  ownedRun,
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

// jsdom has no layout, so it has no `scrollIntoView`; the timeline brings a task row into view.
const scrollIntoView = vi.fn();
const original = Element.prototype.scrollIntoView;
beforeAll(() => {
  Element.prototype.scrollIntoView = scrollIntoView;
});
afterAll(() => {
  Element.prototype.scrollIntoView = original;
});

/** The viewer's task status row in the timeline. */
function taskRow(): HTMLElement {
  const row = document.querySelector<HTMLElement>('.crew-task-row');
  if (!row) throw new Error('No task row in the timeline.');
  return row;
}

describe('the one task highlight the layout owns (ui-redesign-spec, “The timeline”)', () => {
  let daemon: ScriptedDaemon;

  beforeEach(() => {
    vi.clearAllMocks();
    daemon = installDaemon({ messages: richMessages() });
  });

  it('brings a task the person just started into view once the observer reports it', async () => {
    const messages = richMessages().filter((message) => !message.run_id);
    daemon = installDaemon({ messages });
    daemon.state.http = (path, method) => {
      if (path === `/connections/${connection.id}/runs` && method === 'POST') {
        // The daemon accepted the start; its next observation reports the new run.
        daemon.state.runs = [ownedRun];
        return { run_id: ownedRun.run_id, session_id: ownedRun.session_id };
      }
      return undefined;
    };
    renderCrew();
    await channelReady();
    expect(document.querySelector('.crew-task-row')).toBeNull();

    const user = userEvent.setup();
    await user.click(screen.getByRole('button', { name: 'Ask my agent' }));
    fireEvent.change(await screen.findByLabelText('Task'), {
      target: { value: 'plot counts by sample' },
    });
    await chooseModel('fixture-model');
    await user.click(screen.getByRole('button', { name: 'Start my agent and allow posting here' }));

    await waitFor(() => expect(taskRow()).toHaveClass('crew-highlight'));
    expect(scrollIntoView).toHaveBeenCalled();
  });

  it('highlights the task an Agents row points at', async () => {
    daemon.state.runs = [ownedRun];
    renderCrew();
    await channelReady();
    expect(taskRow()).not.toHaveClass('crew-highlight');

    const agents = screen.getByTestId('crew-agents-section');
    await userEvent.setup().click(within(agents).getByRole('button', { name: /#general/ }));

    await waitFor(() => expect(taskRow()).toHaveClass('crew-highlight'));
  });
});
