import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { composerCopy } from '../composer/copy';
import { installResizeObserverStub } from '../test/crewTestUtils';
import {
  channelReady,
  ids,
  installDaemon,
  mocked,
  renderCrew,
  richMessages,
  richSnapshot,
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
 * Live QA round 3, Q3-09. A channel the person left an unsent draft in now says so in the rail: a
 * pencil where the unread count goes (a count wins) and ", draft" in the row's name. The channel on
 * screen never does — its draft is in the composer — and the mark goes once the draft is sent.
 */

function rail(): HTMLElement {
  return screen.getByRole('navigation', { name: 'Crew' });
}

/** The rail row of `#name`, found by its data key whatever its accessible name says. */
function row(channelId: string): HTMLElement {
  const found = rail().querySelector<HTMLElement>(`[data-crew-row="channel:${channelId}"]`);
  if (!found) throw new Error(`No rail row for ${channelId}.`);
  return found;
}

function marker(channelId: string): Element | null {
  return row(channelId).querySelector('[data-testid="crew-channel-draft-marker"]');
}

let daemon: ScriptedDaemon;

beforeEach(() => {
  vi.clearAllMocks();
  window.localStorage.clear();
  daemon = installDaemon({ messages: richMessages() });
});

describe('the rail marks a channel holding an unsent draft (Q3-09)', () => {
  it('shows after switching away from a typed channel, and goes once that draft is sent', async () => {
    renderCrew();
    const composer = await channelReady('general');
    expect(marker(ids.general)).toBeNull();

    // Typing marks nothing: the draft is in the composer, on screen.
    fireEvent.change(composer, { target: { value: 'half-written reply' } });
    expect(marker(ids.general)).toBeNull();
    expect(row(ids.general)).toHaveAccessibleName('general');

    fireEvent.click(row(ids.methods));
    await channelReady('methods');
    await waitFor(() => expect(marker(ids.general)).not.toBeNull());
    expect(marker(ids.general)).toHaveAttribute('width', '14');
    expect(marker(ids.general)).toHaveAttribute('height', '14');
    expect(marker(ids.general)).toHaveAttribute('aria-hidden', 'true');
    expect(marker(ids.general)).toHaveClass('text-text-muted');
    expect(row(ids.general)).toHaveAccessibleName('general, draft');
    expect(row(ids.general)).toHaveAttribute('data-draft', 'true');
    // The channel on screen has nothing kept.
    expect(marker(ids.methods)).toBeNull();
    expect(row(ids.methods)).toHaveAccessibleName('methods');

    // Back to #general: the draft comes back into the composer, and the row is the current one.
    fireEvent.click(row(ids.general));
    expect(await channelReady('general')).toHaveValue('half-written reply');
    expect(marker(ids.general)).toBeNull();
    expect(row(ids.general)).toHaveAccessibleName('general');

    fireEvent.click(screen.getByRole('button', { name: composerCopy.send }));
    await waitFor(() =>
      expect(mocked.crewRequest.mock.calls.some(([, method]) => method === 'message.post')).toBe(
        true
      )
    );
    await waitFor(() =>
      expect(screen.getByRole('textbox', { name: 'Message #general' })).toHaveValue('')
    );

    fireEvent.click(row(ids.methods));
    await channelReady('methods');
    expect(marker(ids.general)).toBeNull();
    expect(row(ids.general)).toHaveAccessibleName('general');
  });

  it('lets an unread count win the slot, and still says draft in the name', async () => {
    renderCrew();
    const composer = await channelReady('general');
    fireEvent.change(composer, { target: { value: 'half-written reply' } });
    fireEvent.click(row(ids.methods));
    await channelReady('methods');
    await waitFor(() => expect(marker(ids.general)).not.toBeNull());

    daemon.state.snapshot = richSnapshot({ unread: { [ids.general]: 3 } });
    act(() => daemon.emitState());

    await waitFor(() => expect(marker(ids.general)).toBeNull());
    expect(within(row(ids.general)).getByText('3')).toBeInTheDocument();
    expect(row(ids.general)).toHaveAccessibleName('general, 3 unread, draft');
  });
});
