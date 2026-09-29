import { act, fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { composerCopy } from '../composer/copy';
import { CrewHttpError } from '../crewApi';
import { clearBlobCache } from '../files/blobMetadataCache';
import { crewActionCopy } from '../state/copy';
import { installResizeObserverStub } from '../test/crewTestUtils';
import { timelineCopy } from '../timeline/copy';
import {
  channelReady,
  currentCrew,
  ids,
  installDaemon,
  mocked,
  renderCrew,
  richSnapshot,
  type ScriptedDaemon,
} from './harness';

vi.mock('../crewApi', async () => {
  const actual = await vi.importActual<typeof import('../crewApi')>('../crewApi');
  return { ...actual, crewHttp: vi.fn(), crewRequest: vi.fn(), observeCrew: vi.fn() };
});
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
 * RENDERER-4, in the real layout: Send in #general, then open #methods before the broker answers.
 *
 * The send state was one global key. #methods' box turned read-only with a spinner on its Send
 * until #general's post settled, and a refusal of #general's post was shown above #methods' box
 * as "Couldn't send.", as if the person's #methods text had failed.
 */

let answerPost: { resolve(value: unknown): void; reject(reason: unknown): void };
let daemon: ScriptedDaemon;

beforeEach(() => {
  vi.clearAllMocks();
  window.localStorage.clear();
  clearBlobCache();
  let posts = 0;
  daemon = installDaemon({
    snapshot: richSnapshot({ pending_joins: [] }),
    request: (method) => {
      if (method !== 'message.post') return undefined;
      posts += 1;
      if (posts > 1) return { sequence: `posted-${posts}` };
      return new Promise((resolve, reject) => {
        answerPost = { resolve, reject };
      });
    },
  });
});

async function sendInGeneralThenOpenMethods() {
  renderCrew();
  const general = await channelReady('general');
  fireEvent.change(general, { target: { value: 'for #general' } });
  fireEvent.click(screen.getByRole('button', { name: composerCopy.send }));
  await waitFor(() =>
    expect(mocked.crewRequest.mock.calls.some(([, method]) => method === 'message.post')).toBe(true)
  );
  act(() => currentCrew().selectChannel(ids.methods));
  return channelReady('methods');
}

describe('a post on its way when the person opens another channel (RENDERER-4)', () => {
  it('leaves the other channel’s box writable, with nothing sending there', async () => {
    const methods = await sendInGeneralThenOpenMethods();
    expect(methods).not.toHaveAttribute('readonly');
    expect(methods).not.toHaveAttribute('aria-busy');
    expect(screen.queryByText(timelineCopy.sending)).toBeNull();
    fireEvent.change(methods, { target: { value: 'for #methods' } });
    expect(methods).toHaveValue('for #methods');
    await act(async () => answerPost.resolve({ sequence: 'posted-1' }));
    expect(methods).toHaveValue('for #methods');
  });

  it('names the channel whose post was refused, and never puts it above the other box', async () => {
    const methods = await sendInGeneralThenOpenMethods();
    fireEvent.change(methods, { target: { value: 'for #methods' } });
    await act(async () => answerPost.reject(new CrewHttpError('Slow down.', 429)));
    expect(
      await screen.findByText(crewActionCopy.sendFailedIn('#general', 'Slow down.'))
    ).toBeInTheDocument();
    expect(screen.queryByText(composerCopy.sendErrorLead)).toBeNull();
    expect(methods).toHaveValue('for #methods');
  });
});

/**
 * QA M5 and R-4: a send failure outlived its draft and followed the person. Clearing the draft left
 * "Couldn't send. …" above an empty box; opening another channel drew it above that channel's
 * empty box; and a failure of the link stayed in red under a green "Connected" once the connection
 * came back. It belongs to its channel and its draft now.
 */
describe('a send failure belongs to its channel and its draft (QA M5, R-4)', () => {
  /** Every post is refused at once with `failure`. */
  function refuseEveryPost(failure: CrewHttpError) {
    daemon.state.request = (method) => {
      if (method !== 'message.post') return undefined;
      return Promise.reject(failure);
    };
  }

  async function failInGeneral(failure = new CrewHttpError('Slow down.', 429)) {
    refuseEveryPost(failure);
    renderCrew();
    const general = await channelReady('general');
    fireEvent.change(general, { target: { value: 'for #general' } });
    fireEvent.click(screen.getByRole('button', { name: composerCopy.send }));
    const lead = await screen.findByText(composerCopy.sendErrorLead);
    return { general, note: lead.closest('.crew-compose-note') as HTMLElement };
  }

  it('hides it in another channel, and brings it back with the draft', async () => {
    const { note } = await failInGeneral();
    expect(note).toHaveTextContent('Slow down.');
    act(() => currentCrew().selectChannel(ids.methods));
    await channelReady('methods');
    expect(screen.queryByText(composerCopy.sendErrorLead)).toBeNull();
    expect(screen.queryByText('Slow down.')).toBeNull();

    act(() => currentCrew().selectChannel(ids.general));
    const general = await channelReady('general');
    expect(general).toHaveValue('for #general');
    expect(await screen.findByText(composerCopy.sendErrorLead)).toBeInTheDocument();
    expect(screen.getByText('Slow down.')).toBeInTheDocument();
  });

  it('goes when the draft is edited', async () => {
    const { general } = await failInGeneral();
    fireEvent.change(general, { target: { value: 'for #general, again' } });
    await waitFor(() => expect(screen.queryByText(composerCopy.sendErrorLead)).toBeNull());
  });

  it('goes when the draft is cleared', async () => {
    const { general } = await failInGeneral();
    fireEvent.change(general, { target: { value: '' } });
    await waitFor(() => expect(screen.queryByText(composerCopy.sendErrorLead)).toBeNull());
  });

  it('drops a failure of the link once the connection verifies again, and keeps an answer', async () => {
    await failInGeneral(new CrewHttpError('The bridge was lost.', 503, 'crew_not_sent'));
    expect(screen.getByText(composerCopy.notSent)).toBeInTheDocument();
    await act(async () => {
      await currentCrew().refresh();
    });
    await channelReady('general');
    await waitFor(() => expect(screen.queryByText(composerCopy.notSent)).toBeNull());

    // A refusal the workspace answered is still news after the same refresh.
    refuseEveryPost(new CrewHttpError('Slow down.', 429));
    fireEvent.click(screen.getByRole('button', { name: composerCopy.send }));
    await screen.findByText('Slow down.');
    await act(async () => {
      await currentCrew().refresh();
    });
    await channelReady('general');
    expect(screen.getByText('Slow down.')).toBeInTheDocument();
  });
});
