/**
 * The composer while a usage limit holds the chat (owner requirement 2): the daemon makes no
 * model request until the limit is raised or lifted, so the composer says so on one line with
 * the way out, and Send refuses by button and by Enter alike. The daemon's answer decides, never
 * a stored notice: an old notice stays in the transcript after the limit is lifted, and the
 * composer only reads the limit again when one lands.
 */
import React from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import type { SessionUsageLimitResponse } from '../api';
import { USAGE_LIMIT_COPY } from './bottom_menu/copy';

const api = vi.hoisted(() => ({ getLimit: vi.fn(), getSummary: vi.fn() }));
// The real dictation hook, except that a test may say a recording holds the composer.
const dictation = vi.hoisted(() => ({ holds: false }));

vi.mock('./ConfigContext', () => ({
  useConfig: () => ({
    getProviders: vi.fn(async () => []),
    read: vi.fn(async () => null),
  }),
}));
vi.mock('./ModelAndProviderContext', () => ({
  useModelAndProvider: () => ({
    getCurrentModelAndProvider: vi.fn(async () => ({
      model: 'gpt-5.6-sol',
      provider: 'versa_azure',
    })),
    currentModel: 'gpt-5.6-sol',
    currentProvider: 'versa_azure',
    modelConfigStatus: 'ready',
    currentModelSupportsVision: false,
    currentModelSupportedInputMimeTypes: null,
  }),
}));
vi.mock('../hooks/useDiverge', () => ({
  useDiverge: () => ({ diverge: vi.fn() }),
}));
vi.mock('./bottom_menu/ToolsChip', () => ({ ToolsChip: () => null }));
vi.mock('./bottom_menu/ModelEffortChip', () => ({ ModelEffortChip: () => null }));
// The footer as what the composer hands it: the cost card's open requests.
vi.mock('./bottom_menu/ComposerFooter', () => ({
  ComposerFooter: ({ usageLimit }: { usageLimit?: { openCardRequest: number } }) => (
    <span data-testid="footer-open-requests">{usageLimit?.openCardRequest ?? 'none'}</span>
  ),
}));
vi.mock('./MessageQueue', () => ({ default: () => null }));
vi.mock('./MentionPopover', () => {
  const MentionPopoverMock = React.forwardRef(() => null);
  MentionPopoverMock.displayName = 'MentionPopoverMock';
  return { default: MentionPopoverMock };
});
vi.mock('../api', () => ({
  getSession: vi.fn(async () => ({ data: { privacy_tier: 'public' } })),
  llamacppStatus: vi.fn(async () => ({ data: {} })),
  ollamaContextWindow: vi.fn(async () => ({ data: { model: 'x', context_window: null } })),
  updateWorkingDir: vi.fn(async () => ({ data: {} })),
  getSessionUsageLimit: api.getLimit,
  getUsageSummary: api.getSummary,
  putSessionUsageLimit: vi.fn(),
}));
vi.mock('./dictation/useDictationComposer', async (importOriginal) => {
  const real = await importOriginal<typeof import('./dictation/useDictationComposer')>();
  return {
    ...real,
    useDictationComposer: (...args: Parameters<typeof real.useDictationComposer>) => {
      const hook = real.useDictationComposer(...args);
      return dictation.holds ? { ...hook, available: true, holdsComposer: true } : hook;
    },
  };
});
vi.mock('../utils/userAction', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../utils/userAction')>()),
  userActionHeaders: async () => ({ 'X-User-Action': 'proof' }),
}));

import ChatInput from './ChatInput';
import { ChatState } from '../types/chatState';

function limitState(overrides: Partial<SessionUsageLimitResponse> = {}): SessionUsageLimitResponse {
  return {
    billingSessionId: 'chat-a',
    chatDollarLimitInert: false,
    chatOverride: null,
    defaults: { maxTokens: null, maxUsd: null },
    limit: { maxTokens: null, maxUsd: null, source: 'none' },
    monthDollarLimitInert: false,
    monthly: { maxTokens: null, maxUsd: null },
    paused: false,
    pausedByEstimate: false,
    reason: null,
    used: {
      chatTokens: 0,
      chatUsdComplete: true,
      month: '2026-11',
      monthTokens: 0,
      monthUsdComplete: true,
    },
    warn: false,
    ...overrides,
  };
}

const pausedBy = (reason: 'chat' | 'month') =>
  limitState({
    limit: { maxTokens: 10_000, maxUsd: null, source: 'chat' },
    paused: true,
    reason,
  });

beforeEach(() => {
  vi.clearAllMocks();
  dictation.holds = false;
  api.getLimit.mockResolvedValue({ data: limitState() });
  api.getSummary.mockResolvedValue({ data: { monthlyReached: false, monthlyNear: false } });
  Object.assign(window, {
    appConfig: { get: () => '/default/workdir' },
    electron: {
      directoryChooser: vi.fn(),
      addRecentDir: vi.fn(),
      logInfo: vi.fn(),
      getPathForFile: vi.fn(() => ''),
      on: vi.fn(),
      off: vi.fn(),
    },
  });
});

const setView = vi.fn();
const handleSubmit = vi.fn();

function composer(
  chatState = ChatState.Idle,
  usageLimitNoticeKey?: string,
  { sessionId = 'chat-a', inputTokens = 0 }: { sessionId?: string; inputTokens?: number } = {}
) {
  return (
    <ChatInput
      sessionId={sessionId}
      handleSubmit={handleSubmit}
      chatState={chatState}
      onStop={vi.fn()}
      initialValue="hello"
      setView={setView}
      totalTokens={0}
      accumulatedInputTokens={inputTokens}
      accumulatedOutputTokens={0}
      droppedFiles={[]}
      onFilesProcessed={vi.fn()}
      messagesLength={2}
      toolCount={0}
      onWorkingDirChange={vi.fn()}
      usageLimitNoticeKey={usageLimitNoticeKey}
    />
  );
}

const send = () => document.querySelector('button[type="submit"]');
// Radix opens a trigger's tooltip on a focus the Tab key moved (`ui/tooltip.tsx`).
const openSendTooltip = () => {
  fireEvent.keyDown(document, { key: 'Tab' });
  fireEvent.focus(send()!.parentElement!);
};

describe('the composer while a usage limit holds the chat', () => {
  it('reads the chat’s limit with the proof', async () => {
    render(composer());
    await waitFor(() =>
      expect(api.getLimit).toHaveBeenCalledWith(
        expect.objectContaining({
          path: { session_id: 'chat-a' },
          headers: { 'X-User-Action': 'proof' },
        })
      )
    );
  });

  it('says so on one line, and refuses to send by button and by Enter alike', async () => {
    api.getLimit.mockResolvedValue({ data: pausedBy('chat') });
    render(composer());
    const notice = await screen.findByTestId('composer-usage-limit-notice');
    expect(notice).toHaveTextContent(USAGE_LIMIT_COPY.noticeChat);
    expect(notice).toHaveAttribute('role', 'status');
    expect(send()).toBeDisabled();
    fireEvent.keyDown(screen.getByRole('textbox'), { key: 'Enter', code: 'Enter' });
    expect(handleSubmit).not.toHaveBeenCalled();
  });

  it('opens the cost card for a chat limit', async () => {
    api.getLimit.mockResolvedValue({ data: pausedBy('chat') });
    render(composer());
    const action = await screen.findByRole('button', { name: USAGE_LIMIT_COPY.noticeChatAction });
    expect(screen.getByTestId('footer-open-requests')).toHaveTextContent('0');
    fireEvent.click(action);
    expect(screen.getByTestId('footer-open-requests')).toHaveTextContent('1');
  });

  it('opens Settings > App > Usage for the monthly limit', async () => {
    api.getLimit.mockResolvedValue({ data: pausedBy('month') });
    render(composer());
    const notice = await screen.findByTestId('composer-usage-limit-notice');
    expect(notice).toHaveTextContent(USAGE_LIMIT_COPY.noticeMonth);
    fireEvent.click(screen.getByRole('button', { name: USAGE_LIMIT_COPY.noticeMonthAction }));
    expect(setView).toHaveBeenCalledWith('settings', { section: 'usage' });
  });

  it('sends as usual when no limit holds the chat', async () => {
    render(composer());
    await waitFor(() => expect(api.getLimit).toHaveBeenCalled());
    expect(screen.queryByTestId('composer-usage-limit-notice')).toBeNull();
    fireEvent.keyDown(screen.getByRole('textbox'), { key: 'Enter', code: 'Enter' });
    expect(handleSubmit).toHaveBeenCalled();
  });

  it('reads the limit again after a turn and when a stored notice lands, and follows the answer', async () => {
    const { rerender } = render(composer(ChatState.Streaming));
    await waitFor(() => expect(api.getLimit).toHaveBeenCalledTimes(1));

    // The turn ends: the gate may have stopped it.
    api.getLimit.mockResolvedValue({ data: pausedBy('chat') });
    rerender(composer(ChatState.Idle));
    await waitFor(() => expect(api.getLimit).toHaveBeenCalledTimes(2));
    await screen.findByTestId('composer-usage-limit-notice');

    // The person lifts the limit elsewhere; the old notice stays in the transcript, and a new
    // one landing is only a reason to read again.
    api.getLimit.mockResolvedValue({ data: limitState() });
    rerender(composer(ChatState.Idle, 'notice-1'));
    await waitFor(() => expect(api.getLimit).toHaveBeenCalledTimes(3));
    await waitFor(() => expect(screen.queryByTestId('composer-usage-limit-notice')).toBeNull());
    expect(send()).not.toBeDisabled();
  });

  it('says the next request would pass the limit when that is why the chat is paused', async () => {
    api.getLimit.mockResolvedValue({
      data: { ...pausedBy('chat'), pausedByEstimate: true, nextRequest: { tokens: 36_000 } },
    });
    render(composer());
    const notice = await screen.findByTestId('composer-usage-limit-notice');
    expect(notice).toHaveTextContent(USAGE_LIMIT_COPY.noticeChatNext);
    expect(send()).toBeDisabled();
  });

  it('holds Send in a new chat while the monthly limit is reached', async () => {
    api.getSummary.mockResolvedValue({ data: { monthlyReached: true, monthlyNear: false } });
    render(composer(ChatState.Idle, undefined, { sessionId: '' }));
    const notice = await screen.findByTestId('composer-usage-limit-notice');
    expect(notice).toHaveAttribute('data-reason', 'month');
    expect(notice).toHaveTextContent(USAGE_LIMIT_COPY.noticeMonth);
    expect(api.getSummary).toHaveBeenCalledWith(
      expect.objectContaining({ headers: { 'X-User-Action': 'proof' } })
    );
    expect(api.getLimit).not.toHaveBeenCalled();
    expect(send()).toBeDisabled();
    fireEvent.keyDown(screen.getByRole('textbox'), { key: 'Enter', code: 'Enter' });
    expect(handleSubmit).not.toHaveBeenCalled();
  });

  // Dictation holding the composer outranks the usage limit as Send's reason: the person is
  // mid-recording, and the notice above the text already says the chat is paused.
  it('names the recording, not the limit, while dictation holds the composer', async () => {
    dictation.holds = true;
    api.getLimit.mockResolvedValue({ data: pausedBy('chat') });
    render(composer());
    await screen.findByTestId('composer-usage-limit-notice');
    expect(send()).toBeDisabled();
    openSendTooltip();
    expect((await screen.findAllByText('Finish dictating first')).length).toBeGreaterThan(0);
    expect(screen.queryByText(USAGE_LIMIT_COPY.sendPaused)).toBeNull();
  });

  it('names the limit as Send’s reason once no recording holds the composer', async () => {
    api.getLimit.mockResolvedValue({ data: pausedBy('chat') });
    render(composer());
    await screen.findByTestId('composer-usage-limit-notice');
    openSendTooltip();
    expect((await screen.findAllByText(USAGE_LIMIT_COPY.sendPaused)).length).toBeGreaterThan(0);
  });

  it('reads the limit again when the chat’s usage moves between turns', async () => {
    const { rerender } = render(composer());
    await waitFor(() => expect(api.getLimit).toHaveBeenCalledTimes(1));
    // A title's side call lands after the turn and moves the chat's counters.
    rerender(composer(ChatState.Idle, undefined, { inputTokens: 77 }));
    await waitFor(() => expect(api.getLimit).toHaveBeenCalledTimes(2));
  });
});
