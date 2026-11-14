/**
 * The message editor's microphone (DD-60): immediately left of Save, so Save stays last; Save
 * waits while dictation holds the editor; Escape cancels the dictation, not the edit; and the
 * stream names the editor of this chat.
 */
import type React from 'react';
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
const observations = vi.hoisted(() => ({
  post: vi.fn(async (_options: Record<string, unknown>) => ({})),
}));
vi.mock('../api', async (original) => ({
  ...(await original<typeof import('../api')>()),
  postDictionaryObservations: observations.post,
}));
vi.mock('../utils/userAction', () => ({
  userActionHeaders: async () => ({ 'X-User-Action': 'proof' }),
}));
import UserMessage from './UserMessage';
import type { Message } from '../api';
import type { CaptureOptions } from './dictation/capture';
import type { StreamRequest } from './dictation/dictationApi';
import { resetDictationSessionForTests } from './dictation/dictationSession';
import { resetDictationStoreForTests } from './dictation/dictationStore';
import {
  FakeCapture,
  FakeStream,
  fakeStatusApi,
  secureMediaPage,
  speechStatus,
  transcript,
} from './dictation/dictationTestKit';

const message: Message = {
  id: 'message-1',
  role: 'user',
  created: 1,
  content: [{ type: 'text', text: 'original prompt' }],
  metadata: { userVisible: true, agentVisible: true },
};

let captures: FakeCapture[] = [];
let streams: FakeStream[] = [];

beforeEach(() => {
  observations.post.mockClear();
  secureMediaPage();
  Object.assign(window, { electron: { platform: 'darwin', logInfo: vi.fn() } });
  captures = [];
  streams = [];
  resetDictationStoreForTests(fakeStatusApi(() => speechStatus()).api);
  resetDictationSessionForTests({
    startCapture: (options: CaptureOptions) => {
      const capture = new FakeCapture(options);
      captures.push(capture);
      return Promise.resolve(capture);
    },
    createStream: (request: StreamRequest) => {
      const stream = new FakeStream(request);
      streams.push(stream);
      return stream;
    },
  });
});

afterEach(() => {
  cleanup();
  resetDictationSessionForTests();
  resetDictationStoreForTests();
});

const settle = async () => {
  for (let i = 0; i < 6; i++) await act(async () => {});
};

function openEditor(update: React.ComponentProps<typeof UserMessage>['onMessageUpdate']) {
  render(<UserMessage message={message} onMessageUpdate={update} sessionId="chat-9" />);
  fireEvent.click(screen.getByRole('button', { name: /edit message:/i }));
  return screen.getByRole('textbox', { name: 'Edit message content' }) as HTMLTextAreaElement;
}

async function correctedEditor(
  update: React.ComponentProps<typeof UserMessage>['onMessageUpdate']
) {
  const field = openEditor(update);
  field.setSelectionRange(0, field.value.length);
  fireEvent.pointerDown(screen.getByTestId('dictation-button'), { button: 0 });
  await settle();
  act(() => captures[0].live());
  act(() => captures[0].speak(1));
  fireEvent.keyDown(field, { key: 'Enter' });
  await act(async () =>
    streams[0].finishDeferred.resolve(transcript('Check the a pass one gene in the lung'))
  );
  await settle();
  fireEvent.change(field, { target: { value: 'Check the EPAS1 gene in the lung' } });
  await settle();
  return field;
}

it('ordinary editor saves learn only after true acceptance', async () => {
  let accept!: (result: boolean) => void;
  const update = vi.fn(
    () =>
      new Promise<boolean>((resolve) => {
        accept = resolve;
      })
  );
  await correctedEditor(update);
  fireEvent.click(screen.getByRole('button', { name: 'Save' }));
  expect(observations.post).not.toHaveBeenCalled();
  await act(async () => accept(true));
  await settle();
  expect(observations.post).toHaveBeenCalledWith({
    body: {
      session_id: 'chat-9',
      observations: [
        { record_id: 'stream-1', kind: 'correction', from: 'a pass one', to: 'EPAS1' },
      ],
    },
    headers: { 'X-User-Action': 'proof' },
  });
});

it.each(['refused', 'failed', 'canceled', 'diverge'] as const)(
  '%s editor sends teach nothing',
  async (kind) => {
    let accept!: (result: boolean) => void;
    const update = vi.fn(() =>
      kind === 'failed'
        ? Promise.reject(new Error('failed'))
        : new Promise<boolean>((resolve) => {
            accept = resolve;
          })
    );
    await correctedEditor(update);
    fireEvent.click(
      screen.getByRole('button', { name: kind === 'diverge' ? 'Save as new chat' : 'Save' })
    );
    if (kind === 'canceled')
      fireEvent.click(screen.getByRole('button', { name: 'Cancel editing' }));
    if (kind !== 'failed') await act(async () => accept(kind !== 'refused'));
    await settle();
    expect(observations.post).not.toHaveBeenCalled();
    if (kind === 'refused' || kind === 'failed') {
      expect(screen.getByRole('textbox', { name: 'Edit message content' })).toBeInTheDocument();
    }
  }
);
