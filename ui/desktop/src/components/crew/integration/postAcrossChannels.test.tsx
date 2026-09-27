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

beforeEach(() => {
  vi.clearAllMocks();
  window.localStorage.clear();
  clearBlobCache();
  let posts = 0;
  installDaemon({
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
