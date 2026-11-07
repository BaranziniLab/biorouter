/**
 * The message editor's microphone (DD-60): immediately left of Save, so Save stays last; Save
 * waits while dictation holds the editor; Escape cancels the dictation, not the edit; and the
 * stream names the editor of this chat.
 */
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import UserMessage from './UserMessage';
import type { Message } from '../api';
import type { CaptureOptions } from './dictation/capture';
import type { StreamRequest } from './dictation/dictationApi';
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

function openEditor() {
  render(<UserMessage message={message} onMessageUpdate={vi.fn()} sessionId="chat-9" />);
  fireEvent.click(screen.getByRole('button', { name: /edit message:/i }));
  return screen.getByRole('textbox', { name: 'Edit message content' }) as HTMLTextAreaElement;
}

describe('the message editor’s microphone', () => {
  it('sits immediately left of Save, which stays last', () => {
    openEditor();
    const save = screen.getByRole('button', { name: 'Save' });
    const footer = save.closest('.justify-end') as HTMLElement;
    expect(footer.lastElementChild?.contains(save) || footer.lastElementChild === save).toBe(true);
    const mic = screen.getByTestId('dictation-button');
    const micSlot = footer.querySelector('.br-dictation-slot')!;
    expect(micSlot.contains(mic)).toBe(true);
    expect(micSlot.compareDocumentPosition(save) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it('names the editor of this chat, holds Save while dictating, and inserts at the caret', async () => {
    const field = openEditor();
    field.setSelectionRange(field.value.length, field.value.length);
    fireEvent.pointerDown(screen.getByTestId('dictation-button'), { button: 0 });
    await settle();
    expect(streams[0].request).toMatchObject({ target_kind: 'editor', session_id: 'chat-9' });
    act(() => captures[0].live());
    act(() => captures[0].speak(1));
    expect(screen.getByRole('button', { name: 'Save' })).toBeDisabled();
    // Enter stops and inserts; it never saves.
    fireEvent.keyDown(field, { key: 'Enter', metaKey: true });
    expect(getDictationSnapshot().activity).toBe('transcribing');
    await act(async () => streams[0].finishDeferred.resolve(transcript('with EPAS1')));
    await settle();
    expect(field.value).toBe('original prompt with EPAS1');
    expect(screen.getByRole('button', { name: 'Save' })).not.toBeDisabled();
  });

  it('sends at most 2 KiB of UTF-8 before the caret as context, cut on a character boundary', async () => {
    // 1,502 UTF-16 units of Greek, CJK and an emoji are 3,604 bytes of UTF-8: the old
    // 2,048 unit slice sent all of it, and the daemon answers 413 above 2,048 bytes.
    const prefix = 'αβγ'.repeat(300) + '基因表达'.repeat(150) + '😀';
    const long: Message = { ...message, content: [{ type: 'text', text: prefix }] };
    render(<UserMessage message={long} onMessageUpdate={vi.fn()} sessionId="chat-9" />);
    fireEvent.click(screen.getByRole('button', { name: /edit message:/i }));
    const field = screen.getByRole('textbox', {
      name: 'Edit message content',
    }) as HTMLTextAreaElement;
    expect(field.value).toBe(prefix);
    field.setSelectionRange(field.value.length, field.value.length);
    fireEvent.pointerDown(screen.getByTestId('dictation-button'), { button: 0 });
    await settle();
    const context = String(streams[0].request.context);
    expect(new TextEncoder().encode(context).length).toBeLessThanOrEqual(2048);
    expect(new TextEncoder().encode(context).length).toBeGreaterThan(2040);
    // The end nearest the caret, with no half of a surrogate pair at either end.
    expect(prefix.endsWith(context)).toBe(true);
    expect(context.endsWith('😀')).toBe(true);
    expect(/^[\uDC00-\uDFFF]/.test(context)).toBe(false);
  });

  it('Escape while recording cancels the dictation and keeps the editor open', async () => {
    const field = openEditor();
    fireEvent.pointerDown(screen.getByTestId('dictation-button'), { button: 0 });
    await settle();
    act(() => captures[0].live());
    fireEvent.keyDown(field, { key: 'Escape' });
    expect(getDictationSnapshot().activity).toBe('idle');
    expect(screen.getByRole('textbox', { name: 'Edit message content' })).toBeInTheDocument();
  });
});
