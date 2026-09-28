import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { composerCopy } from '../composer/copy';
import { installResizeObserverStub } from '../test/crewTestUtils';
import {
  channelReady,
  currentCrew,
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

/**
 * QA M10: being removed from a channel cleared the person's unsent words, and a draft kept for a
 * channel they had moved away from went without a word. The words are offered once now, in a note
 * above the message box with Copy draft, held only by that note, and never put in a composer.
 */
describe('the unsent words of a channel the person loses (QA M10)', () => {
  /** The workspace as it is once #general is closed to the viewer. */
  function withoutGeneral() {
    const snapshot = richSnapshot();
    daemon.state.snapshot = {
      ...snapshot,
      channels: snapshot.channels.filter((item) => item.id !== ids.general),
    };
  }
  const lostNote = () =>
    screen.getByText(/held your unsent draft/).closest('[role="status"]') as HTMLElement;

  it('offers the words in the composer for copying, and drops them when the note is closed', async () => {
    const writeText = vi.fn(async () => undefined);
    Object.assign(navigator, { clipboard: { writeText } });
    renderCrew();
    const general = await channelReady('general');
    fireEvent.change(general, { target: { value: 'half-written reply' } });
    withoutGeneral();
    act(() => daemon.emitState());

    const methods = await channelReady('methods');
    expect(methods).toHaveValue('');
    const note = lostNote();
    expect(note).toHaveTextContent('#general held your unsent draft');
    fireEvent.click(within(note).getByRole('button', { name: 'Copy draft' }));
    await waitFor(() => expect(writeText).toHaveBeenCalledWith('half-written reply'));
    expect(within(note).getByRole('button', { name: 'Copied' })).toBeInTheDocument();

    fireEvent.click(within(note).getByRole('button', { name: 'Dismiss' }));
    expect(screen.queryByText(/held your unsent draft/)).toBeNull();
    expect(methods).toHaveValue('');
  });

  it('offers a draft kept for a channel the person had moved away from, too', async () => {
    renderCrew();
    const general = await channelReady('general');
    fireEvent.change(general, { target: { value: 'kept for later' } });
    act(() => currentCrew().selectChannel(ids.methods));
    const methods = await channelReady('methods');
    withoutGeneral();
    act(() => daemon.emitState());
    await waitFor(() => expect(lostNote()).toHaveTextContent('#general held your unsent draft'));
    expect(methods).toHaveValue('');
  });
});
