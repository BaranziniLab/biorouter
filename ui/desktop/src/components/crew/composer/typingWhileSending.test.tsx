import { act, cleanup, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useRef, useState } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  crewTestController,
  CrewTestProvider,
  testChannel,
  testSnapshot,
} from '../files/crewTestController';
import { useCrewActions } from '../state/crewActions';
import { bodyAfterSending, createSend, useCrewDraft, useOpenSendScreen } from '../state/crewSend';
import { Composer } from './Composer';

/**
 * QA M8, adopted from the reproduction's own test (`evidence/repro-M8/M8Repro.test.tsx`): the real
 * Composer over the real `useCrewActions`, `useCrewDraft` and `createSend`, with only the broker's
 * answer held by hand. The text box was read-only while a post was on its way, so every key typed
 * between Enter and the answer was dropped, and a success then cleared the whole box, so making it
 * writable alone would have lost the same words a moment later.
 */

vi.mock('../crewTransfers', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../crewTransfers')>()),
  beginTransfer: vi.fn(),
  listTransfers: vi.fn(async () => []),
  pauseTransfer: vi.fn(),
  resumeTransfer: vi.fn(),
  forgetTransfer: vi.fn(),
  previewAttachment: vi.fn(),
  clearPublishedTransfers: vi.fn(async () => undefined),
}));

interface Held {
  promise: Promise<unknown>;
  resolve(value: unknown): void;
}

function held(): Held {
  let resolve!: (value: unknown) => void;
  const promise = new Promise<unknown>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

function Harness({ request }: { request: (method: string, params?: unknown) => Promise<unknown> }) {
  const actions = useCrewActions();
  const draft = useCrewDraft();
  const generation = useRef(0);
  const historyPage = useRef<string | null>(null);
  const [, setHistoryBefore] = useState<string | null>(null);
  const selection = useRef({ connectionId: 'connection-1', channelId: testChannel.id });
  useOpenSendScreen(selection, actions.reportError);
  const send = createSend({
    draft,
    busy: actions.busy,
    connectionId: 'connection-1',
    channelId: testChannel.id,
    channel: testChannel,
    snapshot: testSnapshot,
    observedPrivacy: {
      connectionId: 'connection-1',
      mode: 'private',
      institutionId: 'ucsf',
      policyEpoch: 1,
    },
    generation,
    selection,
    historyPage,
    setHistoryBefore,
    restartObservation: () => undefined,
    request: request as never,
    markRead: async () => undefined,
    act: actions.act,
    reportError: actions.reportError,
  });
  const controller = crewTestController({
    draft: { body: draft.body, attachments: draft.attachments, references: draft.references },
    setBody: draft.setBody,
    send,
    act: actions.act,
    isPending: actions.isPending,
    busy: actions.busy,
    error: actions.error,
  });
  return (
    <CrewTestProvider controller={controller}>
      <Composer />
    </CrewTestProvider>
  );
}

afterEach(cleanup);

describe('typing while a post is on its way (QA M8)', () => {
  it('keeps what is typed after Enter, and takes out only what was sent', async () => {
    const answer = held();
    const posted: { body: string }[] = [];
    const request = vi.fn(async (_method: string, params?: unknown) => {
      posted.push(params as { body: string });
      return answer.promise;
    });
    render(<Harness request={request} />);
    const user = userEvent.setup({ delay: null });
    const input = screen.getByLabelText('Message #general') as HTMLTextAreaElement;

    await user.click(input);
    await user.keyboard('hello');
    await user.keyboard('{Enter}');
    expect(request).toHaveBeenCalledTimes(1);
    // On its way: the box is writable and keeps what is typed now.
    expect(input.readOnly).toBe(false);
    await user.keyboard(' and more');
    expect(input).toHaveValue('hello and more');

    await act(async () => {
      answer.resolve({ sequence: '42', id: 'message-1' });
    });
    await waitFor(() => expect(input).toHaveValue(' and more'));
    expect(posted[0].body).toBe('hello');
  });

  it('clears the box when nothing was typed after Enter', async () => {
    const request = vi.fn(async () => ({ sequence: '1', id: 'message-1' }));
    render(<Harness request={request} />);
    const user = userEvent.setup({ delay: null });
    const input = screen.getByLabelText('Message #general') as HTMLTextAreaElement;
    await user.click(input);
    await user.keyboard('hello{Enter}');
    await waitFor(() => expect(input).toHaveValue(''));
  });
});

describe('bodyAfterSending', () => {
  it.each([
    ['all of it was sent', 'hello', 'hello', ''],
    ['more was typed after it', 'hello and more', 'hello', ' and more'],
    ['only spaces were typed after it', 'hello  ', 'hello', ''],
    ['the sent words were edited meanwhile', 'Hello', 'hello', 'Hello'],
    ['only files were sent', 'typed later', '', 'typed later'],
  ])('when %s', (_label, current, sent, expected) => {
    expect(bodyAfterSending(current, sent)).toBe(expected);
  });
});
