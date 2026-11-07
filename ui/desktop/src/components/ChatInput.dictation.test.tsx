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
import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const providers = vi.hoisted(() => ({ rows: [] as Array<Record<string, unknown>> }));
vi.mock('./ConfigContext', () => ({
  useConfig: () => ({
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
  getSession: vi.fn(async () => ({ data: null })),
  llamacppStatus: vi.fn(async () => ({ data: {} })),
  updateWorkingDir: vi.fn(async () => ({ data: {} })),
}));

import ChatInput from './ChatInput';
import { ChatState } from '../types/chatState';
import { COMPOSER_COPY } from './composer/copy';
import type { CaptureOptions } from './dictation/capture';
import type { DictationStatus, StreamRequest } from './dictation/dictationApi';
import { getDictationSnapshot, resetDictationSessionForTests } from './dictation/dictationSession';
import { resetDictationStoreForTests } from './dictation/dictationStore';
import {
  FakeCapture,
  FakeStream,
  fakeStatusApi,
  secureMediaPage,
  speechStatus,
  transcript,
} from './dictation/dictationTestKit';
import { isPrivateDraft, resetPrivateDraftsForTests } from './dictation/privateDraft';
import { readComposerDraft, resetComposerDraftsForTests } from '../utils/composerDrafts';

let captures: FakeCapture[] = [];
let streams: FakeStream[] = [];
let status: DictationStatus | null = null;
let reads: ReturnType<typeof vi.fn>;

beforeEach(() => {
  secureMediaPage();
  providers.rows = [{ name: 'openai', resolved_tier: 'public' }];
  status = speechStatus();
  captures = [];
  streams = [];
  const fake = fakeStatusApi(() => status);
  reads = fake.reads;
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
  const result = render(
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
      {...overrides}
    />
  );
  return { handleSubmit, unmount: result.unmount };
}

const field = () => screen.getByTestId('chat-input') as HTMLTextAreaElement;
const mic = () => screen.getByTestId('dictation-button');
const send = () => screen.getByRole('button', { name: COMPOSER_COPY.send.label });

async function record(seconds = 1) {
  fireEvent.pointerDown(mic(), { button: 0 });
  await settle();
  act(() => captures[0].live());
  act(() => captures[0].speak(seconds));
}

describe('mounting', () => {
  it('sends no status request, even with the field focused at mount', async () => {
    renderComposer();
    await settle();
    expect(document.activeElement).toBe(field());
    expect(reads).not.toHaveBeenCalled();
    // A later focus is a sign of interest.
    act(() => field().blur());
    act(() => field().focus());
    await settle();
    expect(reads).toHaveBeenCalledTimes(1);
  });
});

describe('the submit gates while dictating', () => {
  it('Enter while recording stops and inserts, and never sends', async () => {
    const { handleSubmit } = renderComposer();
    fireEvent.change(field(), { target: { value: 'Check' } });
    await record();
    expect(getDictationSnapshot().activity).toBe('recording');
    fireEvent.keyDown(field(), { key: 'Enter' });
    expect(getDictationSnapshot().activity).toBe('transcribing');
    // Enter while transcribing does nothing at all.
    fireEvent.keyDown(field(), { key: 'Enter' });
    expect(handleSubmit).not.toHaveBeenCalled();
    await act(async () => streams[0].finishDeferred.resolve(transcript('EPAS1')));
    await settle();
    expect(field().value).toBe('Check EPAS1');
    // Nothing is sent by itself (DD-70); the next Enter sends.
    expect(handleSubmit).not.toHaveBeenCalled();
    fireEvent.keyDown(field(), { key: 'Enter' });
    await settle();
    expect(handleSubmit).toHaveBeenCalledTimes(1);
  });

  it('disables Send with the reason, and refuses a form submit, while arming, recording and transcribing', async () => {
    const { handleSubmit } = renderComposer();
    fireEvent.change(field(), { target: { value: 'hello' } });
    expect(send()).not.toBeDisabled();
    fireEvent.pointerDown(mic(), { button: 0 });
    expect(send()).toBeDisabled();
    fireEvent.submit(field().closest('form')!);
    await settle();
    act(() => captures[0].live());
    act(() => captures[0].speak(1));
    expect(send()).toBeDisabled();
    fireEvent.submit(field().closest('form')!);
    fireEvent.pointerDown(mic(), { button: 0 });
    expect(getDictationSnapshot().activity).toBe('transcribing');
    expect(send()).toBeDisabled();
    fireEvent.submit(field().closest('form')!);
    await settle();
    expect(handleSubmit).not.toHaveBeenCalled();
  });

  it('says Listening… only while recording into an empty field', async () => {
    renderComposer();
    await record();
    expect(field()).toHaveAttribute('placeholder', 'Listening…');
    fireEvent.change(field(), { target: { value: 'typed' } });
    expect(field().placeholder).not.toBe('Listening…');
  });

  it('puts the microphone immediately left of Send', () => {
    renderComposer();
    const controls = screen.getByTestId('chat-input-toolbar');
    const last = controls.lastElementChild!;
    expect(within(last as HTMLElement).getByRole('button', { name: COMPOSER_COPY.send.label }));
    expect(last.previousElementSibling?.contains(mic())).toBe(true);
  });
});

describe('MSG-PRIVATE-DRAFT (DD-71)', () => {
  async function dictatePrivately() {
    await record();
    fireEvent.keyDown(field(), { key: 'Enter' });
    await act(async () =>
      streams[0].finishDeferred.resolve(transcript('EGLN1', { bias_scope: 'private' }))
    );
    await settle();
  }

  it('asks before a draft dictated with private terms is sent to a public model', async () => {
    const { handleSubmit } = renderComposer();
    await dictatePrivately();
    fireEvent.keyDown(field(), { key: 'Enter' });
    await settle();
    const dialog = screen.getByRole('dialog');
    expect(dialog).toHaveTextContent(
      'This message was dictated with your private dictionary terms turned on, so it may contain them, and gpt-5.6-sol is a public model. Send it anyway?'
    );
    expect(handleSubmit).not.toHaveBeenCalled();
    fireEvent.click(within(dialog).getByRole('button', { name: 'Send anyway' }));
    await settle();
    expect(handleSubmit).toHaveBeenCalledTimes(1);
  });

  it('sends nothing when the person cancels', async () => {
    const { handleSubmit } = renderComposer();
    await dictatePrivately();
    fireEvent.submit(field().closest('form')!);
    await settle();
    fireEvent.click(within(screen.getByRole('dialog')).getByRole('button', { name: 'Cancel' }));
    await settle();
    expect(handleSubmit).not.toHaveBeenCalled();
    expect(field().value).toBe('EGLN1');
  });

  it('asks before a running turn queues it, too', async () => {
    const { handleSubmit } = renderComposer({ chatState: ChatState.Streaming });
    await dictatePrivately();
    fireEvent.keyDown(field(), { key: 'Enter' });
    await settle();
    expect(screen.getByRole('dialog')).toHaveTextContent('Send it anyway?');
    expect(handleSubmit).not.toHaveBeenCalled();
  });

  it('does not ask when the chat’s model is private, judged by its instance tier', async () => {
    providers.rows = [{ name: 'openai', resolved_tier: 'private' }];
    const { handleSubmit } = renderComposer();
    await dictatePrivately();
    fireEvent.keyDown(field(), { key: 'Enter' });
    await settle();
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(handleSubmit).toHaveBeenCalledTimes(1);
  });

  it('still asks for text dictated while its composer was away, once the person comes back', async () => {
    const draftKey = 'tab:remount';
    const away = renderComposer({ draftKey });
    await record();
    // The person switches tab while still speaking: the composer unmounts, the
    // recording stops, and the text arrives with no composer to put it in.
    away.unmount();
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
    await settle();
    expect(streams[0].finishCalls).toBe(1);
    await act(async () =>
      streams[0].finishDeferred.resolve(transcript('EGLN1', { bias_scope: 'private' }))
    );
    await settle();
    expect(readComposerDraft(draftKey)?.text).toBe('EGLN1');
    expect(isPrivateDraft(draftKey)).toBe(true);
    // Back on the tab, the box shows the stored draft from its first render, and the
    // flag survives the mount.
    const back = renderComposer({ draftKey });
    await settle();
    expect(field().value).toBe('EGLN1');
    expect(isPrivateDraft(draftKey)).toBe(true);
    fireEvent.keyDown(field(), { key: 'Enter' });
    await settle();
    const dialog = screen.getByRole('dialog');
    expect(dialog).toHaveTextContent('Send it anyway?');
    expect(back.handleSubmit).not.toHaveBeenCalled();
    fireEvent.click(within(dialog).getByRole('button', { name: 'Cancel' }));
    await settle();
    // Emptying the draft still clears the flag: the box and the stored draft are both empty.
    fireEvent.change(field(), { target: { value: '' } });
    await settle();
    expect(readComposerDraft(draftKey)?.text ?? '').toBe('');
    expect(isPrivateDraft(draftKey)).toBe(false);
  });

  it('does not ask for text dictated with public terms only', async () => {
    const { handleSubmit } = renderComposer();
    await record();
    fireEvent.keyDown(field(), { key: 'Enter' });
    await act(async () => streams[0].finishDeferred.resolve(transcript('EPAS1')));
    await settle();
    fireEvent.keyDown(field(), { key: 'Enter' });
    await settle();
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(handleSubmit).toHaveBeenCalledTimes(1);
  });
});

describe('the stream target', () => {
  it('names the chat, with the draft before the anchor as context', async () => {
    renderComposer();
    fireEvent.change(field(), { target: { value: 'The gene' } });
    field().setSelectionRange(8, 8);
    await record();
    expect(streams[0].request).toMatchObject({
      target_kind: 'chat',
      session_id: 'chat-1',
      context: 'The gene',
      sample_rate: 16000,
    });
  });

  it('on Home, names the new chat and the tab’s own pick', async () => {
    renderComposer({
      sessionId: null,
      effectiveModel: { provider: 'versa_azure', model: 'gpt-5.5' },
    });
    await record();
    expect(streams[0].request).toMatchObject({
      target_kind: 'new_chat',
      new_chat_provider: 'versa_azure',
      new_chat_model: 'gpt-5.5',
    });
    expect(streams[0].request).not.toHaveProperty('session_id');
  });
});
