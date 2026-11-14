/**
 * The chat composer with its microphone (PR-19, DD-60 to DD-63, DD-70, DD-71):
 * the three submit gates hold while dictation arms, records or transcribes,
 * Enter never sends while recording, text is never sent by itself, and a draft
 * dictated with private terms eligible asks before it leaves for a public model.
 *
 * The session's capture and stream are fakes (`dictation/dictationTestKit.ts`);
 * the composer, the microphone, the session and the status store are real.
 */
import React from 'react';
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';

const observations = vi.hoisted(() => ({
  post: vi.fn(async (_options: Record<string, unknown>) => ({})),
  createSession: vi.fn(),
}));
vi.mock('../utils/userAction', () => ({
  userActionHeaders: async () => ({ 'X-User-Action': 'proof' }),
}));
vi.mock('../sessions', () => ({ createSession: observations.createSession }));
vi.mock('./sessions/SessionsInsights', () => ({ SessionInsights: () => null }));
vi.mock('./privacy/PrivacyTiersOffNote', () => ({ PrivacyTiersOffNote: () => null }));
vi.mock('./privacy/useConfirmNewChatModel', () => ({
  useConfirmNewChatModel: () => async () => true,
}));
const providers = vi.hoisted(() => ({ rows: [] as Array<Record<string, unknown>> }));
vi.mock('./ConfigContext', () => ({
  useConfig: () => ({
    extensionsList: [],
    getProviders: vi.fn(async () => providers.rows),
    read: vi.fn(async () => null),
  }),
}));
vi.mock('./ModelAndProviderContext', () => ({
  useModelAndProvider: () => ({
    getCurrentModelAndProvider: vi.fn(async () => ({ model: null, provider: null })),
    currentModel: 'gpt-5.6-sol',
    currentProvider: 'openai',
    modelConfigStatus: 'ready',
    currentModelSupportsVision: true,
    currentModelSupportedInputMimeTypes: null,
  }),
}));
vi.mock('../hooks/useDiverge', () => ({ useDiverge: () => ({ diverge: vi.fn() }) }));
vi.mock('./bottom_menu/ToolsChip', () => ({
  ToolsChip: () => <button data-testid="tools-chip-stub">Tools</button>,
}));
vi.mock('./bottom_menu/ModelEffortChip', () => ({
  ModelEffortChip: () => <button data-testid="model-chip-stub">gpt-5.6-sol</button>,
}));
vi.mock('./bottom_menu/ComposerFooter', () => ({ ComposerFooter: () => null }));
vi.mock('./MessageQueue', () => ({ default: () => null, canSteerMessage: () => false }));
vi.mock('./MentionPopover', () => {
  const MentionPopoverMock = React.forwardRef(() => null);
  MentionPopoverMock.displayName = 'MentionPopoverMock';
  return { default: MentionPopoverMock };
});
vi.mock('../api', () => ({
  postDictionaryObservations: observations.post,
  getSession: vi.fn(async () => ({ data: null })),
  llamacppStatus: vi.fn(async () => ({ data: {} })),
  updateWorkingDir: vi.fn(async () => ({ data: {} })),
}));

import ChatInput from './ChatInput';
import Hub from './Hub';
import { ChatState } from '../types/chatState';
import type { CaptureOptions } from './dictation/capture';
import type { DictationStatus, StreamRequest } from './dictation/dictationApi';
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
import { resetPrivateDraftsForTests } from './dictation/privateDraft';
import { readComposerDraft, resetComposerDraftsForTests } from '../utils/composerDrafts';

let captures: FakeCapture[] = [];
let streams: FakeStream[] = [];
let status: DictationStatus | null = null;

beforeEach(() => {
  observations.post.mockClear();
  observations.createSession.mockReset();
  localStorage.clear();
  secureMediaPage();
  providers.rows = [{ name: 'openai', resolved_tier: 'public' }];
  status = speechStatus();
  captures = [];
  streams = [];
  const fake = fakeStatusApi(() => status);
  resetDictationStoreForTests(fake.api);
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
  resetPrivateDraftsForTests();
  Object.assign(window, {
    appConfig: { get: () => '/Users/wgu/Desktop' },
    electron: {
      platform: 'darwin',
      directoryChooser: vi.fn(),
      addRecentDir: vi.fn(),
      logInfo: vi.fn(),
      on: vi.fn(),
      off: vi.fn(),
      deleteTempFile: vi.fn(),
      getUserActionKey: vi.fn(async () => 'proof'),
    },
  });
});

afterEach(() => {
  cleanup();
  resetDictationSessionForTests();
  resetDictationStoreForTests();
  resetPrivateDraftsForTests();
  resetComposerDraftsForTests();
});

const settle = async () => {
  for (let i = 0; i < 8; i++) await act(async () => {});
};

function renderComposer(overrides: Partial<React.ComponentProps<typeof ChatInput>> = {}) {
  const handleSubmit = vi.fn(async () => true);
  const view = (extra: typeof overrides = overrides) => (
    <ChatInput
      sessionId="chat-1"
      handleSubmit={handleSubmit}
      chatState={ChatState.Idle}
      onStop={vi.fn()}
      initialValue=""
      setView={vi.fn()}
      totalTokens={0}
      accumulatedInputTokens={0}
      accumulatedOutputTokens={0}
      droppedFiles={[]}
      onFilesProcessed={vi.fn()}
      messagesLength={1}
      toolCount={0}
      onWorkingDirChange={vi.fn()}
      {...extra}
    />
  );
  const result = render(view());
  return {
    handleSubmit,
    ...result,
    rerenderComposer: (extra: typeof overrides) =>
      result.rerender(view({ ...overrides, ...extra })),
  };
}

const field = () => screen.getByTestId('chat-input') as HTMLTextAreaElement;
const mic = () => screen.getByTestId('dictation-button');

async function record(seconds = 1) {
  fireEvent.pointerDown(mic(), { button: 0 });
  await settle();
  act(() => captures[0].live());
  act(() => captures[0].speak(seconds));
}

const source = 'Check the a pass one gene in the lung';
const corrected = 'Check the EPAS1 gene in the lung';

async function dictateSource() {
  await record();
  fireEvent.keyDown(field(), { key: 'Enter' });
  await act(async () =>
    streams[0].finishDeferred.resolve(
      transcript(source, { raw_text: 'RAW_TRANSCRIPT_ONLY_IN_MEMORY' })
    )
  );
  await settle();
  expect(field().value).toBe(source);
}

async function correctSource() {
  await dictateSource();
  fireEvent.change(field(), { target: { value: corrected } });
  await settle();
}

it('posts only the pair after accepted send, with proof and the destination, and no raw ledger', async () => {
  let accept!: (accepted: boolean) => void;
  const submit = vi.fn<React.ComponentProps<typeof ChatInput>['handleSubmit']>(
    () =>
      new Promise<boolean>((resolve) => {
        accept = resolve;
      })
  );
  renderComposer({ handleSubmit: submit });
  await correctSource();
  fireEvent.keyDown(field(), { key: 'Enter' });
  expect(submit).toHaveBeenCalledTimes(1);
  expect(observations.post).not.toHaveBeenCalled();
  await act(async () => accept(true));
  await settle();
  expect(observations.post).toHaveBeenCalledWith({
    body: {
      session_id: 'chat-1',
      observations: [
        { record_id: 'stream-1', kind: 'correction', from: 'a pass one', to: 'EPAS1' },
      ],
    },
    headers: { 'X-User-Action': 'proof' },
  });
  const outgoing = (submit.mock.calls[0][0] as unknown as CustomEvent).detail;
  expect(Object.keys(outgoing).sort()).toEqual(['attachments', 'onAcceptedSession', 'value']);
  expect(JSON.stringify(outgoing)).not.toContain('RAW_TRANSCRIPT');
  expect(JSON.stringify(localStorage)).not.toContain('RAW_TRANSCRIPT');
  expect(JSON.stringify(vi.mocked(window.electron.logInfo).mock.calls)).not.toContain(
    'RAW_TRANSCRIPT'
  );
});

it.each(['refused', 'failed'] as const)(
  '%s sends post nothing and retain their ledger for retry',
  async (failure) => {
    const submit = vi.fn<React.ComponentProps<typeof ChatInput>['handleSubmit']>(() =>
      failure === 'refused' ? Promise.resolve(false) : Promise.reject(new Error('failed'))
    );
    renderComposer({ handleSubmit: submit, draftKey: 'tab:retry' });
    await correctSource();
    fireEvent.keyDown(field(), { key: 'Enter' });
    await settle();
    expect(observations.post).not.toHaveBeenCalled();
    expect(field().value).toBe(corrected);
    expect(readComposerDraft('tab:retry')?.dictation?.[0].state).toBe('edited');
  }
);

it('a canceled private-send warning posts nothing', async () => {
  renderComposer();
  await record();
  fireEvent.keyDown(field(), { key: 'Enter' });
  await act(async () =>
    streams[0].finishDeferred.resolve(transcript(source, { bias_scope: 'private' }))
  );
  await settle();
  fireEvent.change(field(), { target: { value: corrected } });
  fireEvent.keyDown(field(), { key: 'Enter' });
  await settle();
  fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
  expect(field().value).toBe(corrected);
  expect(observations.post).not.toHaveBeenCalled();
});

it('a later programmatic replacement cannot teach a stale pair', async () => {
  const mounted = renderComposer();
  await correctSource();
  mounted.rerenderComposer({ initialValue: 'Check the HIF2-alpha gene in the lung' });
  await settle();
  fireEvent.keyDown(field(), { key: 'Enter' });
  await settle();
  expect(observations.post).not.toHaveBeenCalled();
});

it('a remount retains the edited dictation without putting its raw text on the send', async () => {
  const mounted = renderComposer({ draftKey: 'tab:remount' });
  await correctSource();
  mounted.unmount();
  renderComposer({ draftKey: 'tab:remount' });
  expect(field().value).toBe(corrected);
  fireEvent.keyDown(field(), { key: 'Enter' });
  await settle();
  expect(observations.post).toHaveBeenCalledTimes(1);
});

it('a recording completed after unmount keeps metadata in the returning draft', async () => {
  const mounted = renderComposer({ draftKey: 'tab:recording' });
  await record();
  mounted.unmount();
  await act(async () => new Promise((resolve) => setTimeout(resolve, 0)));
  await act(async () => streams[0].finishDeferred.resolve(transcript(source)));
  await settle();
  expect(readComposerDraft('tab:recording')?.dictation?.[0]).toMatchObject({
    recordId: 'stream-1',
    raw: source,
  });
  renderComposer({ draftKey: 'tab:recording' });
  fireEvent.change(field(), { target: { value: corrected } });
  fireEvent.keyDown(field(), { key: 'Enter' });
  await settle();
  expect(observations.post).toHaveBeenCalledTimes(1);
});

it('Home posts to the chat it created even when its composer has unmounted', async () => {
  let created!: (session: { id: string }) => void;
  observations.createSession.mockImplementation(
    () =>
      new Promise((resolve) => {
        created = resolve;
      })
  );
  const mounted = render(<Hub setView={vi.fn()} />);
  await correctSource();
  fireEvent.keyDown(field(), { key: 'Enter' });
  await settle();
  expect(observations.createSession).toHaveBeenCalledTimes(1);
  mounted.unmount();
  await act(async () => created({ id: 'actual-home-destination' }));
  await settle();
  expect((observations.post.mock.calls[0][0].body as { session_id: string }).session_id).toBe(
    'actual-home-destination'
  );
});

it('queueing retains the ledger until the actual accepted send', async () => {
  const mounted = renderComposer({ chatState: ChatState.Thinking, draftKey: 'tab:queue' });
  await correctSource();
  fireEvent.keyDown(field(), { key: 'Enter' });
  await settle();
  expect(mounted.handleSubmit).not.toHaveBeenCalled();
  expect(observations.post).not.toHaveBeenCalled();
  mounted.rerenderComposer({ chatState: ChatState.Idle });
  await settle();
  expect(mounted.handleSubmit).toHaveBeenCalledTimes(1);
  expect(observations.post).toHaveBeenCalledTimes(1);
});

it('an accepted steer teaches once after acceptance', async () => {
  const steer = vi.fn(async () => true);
  renderComposer({ chatState: ChatState.Thinking, onSteer: steer });
  await correctSource();
  fireEvent.keyDown(field(), { key: 'Enter', metaKey: true });
  await settle();
  expect(steer).toHaveBeenCalledWith(corrected);
  expect(observations.post).toHaveBeenCalledTimes(1);
});
