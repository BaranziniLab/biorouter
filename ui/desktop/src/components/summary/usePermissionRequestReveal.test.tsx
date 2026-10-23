import { act, cleanup, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Message } from '../../api';
import { ChatState } from '../../types/chatState';
import type { ScrollAreaHandle } from '../ui/scroll-area';
import { usePermissionRequestReveal } from './usePermissionRequestReveal';

function message(confirmationId?: string): Message {
  return {
    id: 'assistant-row',
    role: 'assistant',
    created: 1,
    metadata: { userVisible: true, agentVisible: true },
    content: confirmationId
      ? [
          {
            type: 'actionRequired',
            data: {
              actionType: 'toolConfirmation',
              id: confirmationId,
              toolName: 'developer__shell',
              arguments: {},
            },
          },
        ]
      : [{ type: 'text', text: 'Preparing the tool call.' }],
  };
}

interface Props {
  sessionId: string;
  messages: Message[];
  chatState: ChatState;
  transcriptMounted: boolean;
}

const initial: Props = {
  sessionId: 'chat-one',
  messages: [message()],
  chatState: ChatState.Streaming,
  transcriptMounted: true,
};

let frames: Map<number, FrameRequestCallback>;
let frameId: number;

beforeEach(() => {
  frames = new Map();
  frameId = 0;
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => {
    frames.set(++frameId, callback);
    return frameId;
  });
  vi.stubGlobal('cancelAnimationFrame', (id: number) => frames.delete(id));
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

function flushFrames() {
  act(() => {
    const pending = [...frames.values()];
    frames.clear();
    for (const callback of pending) callback(0);
  });
}

function mount(props: Props = initial) {
  const scrollToBottom = vi.fn();
  const scrollRef = {
    current: { scrollToBottom, isFollowing: false } as unknown as ScrollAreaHandle | null,
  };
  return {
    ...renderHook((next: Props) => usePermissionRequestReveal({ ...next, scrollRef }), {
      initialProps: props,
    }),
    scrollToBottom,
    scrollRef,
  };
}

const waiting = (id: string): Props => ({
  ...initial,
  messages: [message(id)],
  chatState: ChatState.WaitingForUserInput,
});

describe('new permission request visibility', () => {
  it('reveals an approval appended to the same message after its committed layout', () => {
    const { rerender, scrollToBottom } = mount();
    rerender(waiting('approval-one'));
    expect(scrollToBottom).not.toHaveBeenCalled();
    flushFrames();
    expect(scrollToBottom.mock.calls).toEqual([['auto']]);
  });

  it('does not pull the reader down again when the same approval rerenders', () => {
    const { rerender, scrollToBottom } = mount();
    rerender(waiting('approval-one'));
    flushFrames();
    rerender(waiting('approval-one'));
    flushFrames();
    expect(scrollToBottom).toHaveBeenCalledTimes(1);
  });

  it('reveals the next approval even while the chat remains waiting', () => {
    const { rerender, scrollToBottom } = mount();
    rerender(waiting('approval-one'));
    flushFrames();
    rerender({
      ...waiting('approval-two'),
      messages: [message('approval-one'), { ...message('approval-two'), id: 'next-row' }],
    });
    flushFrames();
    expect(scrollToBottom.mock.calls).toEqual([['auto'], ['auto']]);
  });

  it('coalesces approvals that arrive before the pending reveal frame', () => {
    const { rerender, scrollToBottom } = mount();
    rerender(waiting('approval-one'));
    rerender(waiting('approval-two'));
    flushFrames();
    expect(scrollToBottom).toHaveBeenCalledTimes(1);
  });

  it('does not treat an initially loaded confirmation as a new live request', () => {
    const { rerender, scrollToBottom } = mount(waiting('historical-approval'));
    flushFrames();
    rerender(waiting('historical-approval'));
    flushFrames();
    expect(scrollToBottom).not.toHaveBeenCalled();
  });

  it('does not reveal history that hydrates while the conversation is loading', () => {
    const { rerender, scrollToBottom } = mount({
      ...initial,
      messages: [],
      chatState: ChatState.LoadingConversation,
    });
    rerender({ ...waiting('historical-approval'), chatState: ChatState.LoadingConversation });
    rerender(waiting('historical-approval'));
    flushFrames();
    expect(scrollToBottom).not.toHaveBeenCalled();
  });

  it('leaves deliberate upward reading in place for ordinary transcript growth', () => {
    const { rerender, scrollToBottom } = mount();
    rerender({ ...initial, messages: [message(), { ...message(), id: 'next-row' }] });
    flushFrames();
    expect(scrollToBottom).not.toHaveBeenCalled();
  });

  it('cancels the reveal when the pending request resolves before the frame', () => {
    const { rerender, scrollToBottom } = mount();
    rerender(waiting('approval-one'));
    rerender({ ...waiting('approval-one'), chatState: ChatState.Idle });
    flushFrames();
    expect(scrollToBottom).not.toHaveBeenCalled();
  });

  it('cancels the old chat reveal when the session switches', () => {
    const { rerender, scrollToBottom } = mount();
    rerender(waiting('approval-one'));
    rerender({ ...waiting('other-chat-approval'), sessionId: 'chat-two' });
    flushFrames();
    expect(scrollToBottom).not.toHaveBeenCalled();
  });

  it('keeps a new request pending until the transcript viewport mounts', () => {
    const { rerender, scrollToBottom, scrollRef } = mount();
    const scroll = scrollRef.current;
    scrollRef.current = null;
    rerender({ ...waiting('approval-one'), transcriptMounted: false });
    flushFrames();
    expect(scrollToBottom).not.toHaveBeenCalled();
    scrollRef.current = scroll;
    rerender(waiting('approval-one'));
    flushFrames();
    expect(scrollToBottom.mock.calls).toEqual([['auto']]);
  });
});
