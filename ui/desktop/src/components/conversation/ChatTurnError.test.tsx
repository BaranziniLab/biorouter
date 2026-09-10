import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { ChatTurnError, hasVisibleTurnErrorMessage, presentChatTurnError } from './ChatTurnError';
import type { Message } from '../../api';
import type { ChatTurnErrorData } from '../../types/turnError';

const QUOTA_ERROR =
  'Stream error: provider_failure: Request failed: Stream decode error: Responses API error: Object {"type": String("insufficient_quota"), "code": String("insufficient_quota"), "message": String("You exceeded your current quota, please check your plan and billing details."), "param": Null}';

function error(overrides: Partial<ChatTurnErrorData> = {}): ChatTurnErrorData {
  return {
    message: 'Provider rejected the request',
    code: 'provider_failure',
    scope: 'provider',
    retryable: false,
    ...overrides,
  };
}

describe('ChatTurnError', () => {
  it('presents known provider categories without depending on raw provider wording', () => {
    expect(presentChatTurnError(error({ providerKind: 'quota' }))).toMatchObject({
      title: 'Model quota exceeded',
      message: 'Provider rejected the request',
    });
    expect(presentChatTurnError(error({ providerKind: 'auth' }))).toMatchObject({
      title: 'Model authentication failed',
    });
  });

  it('extracts a provider message from the reported legacy quota payload', () => {
    expect(
      presentChatTurnError(error({ message: QUOTA_ERROR, technicalDetails: QUOTA_ERROR }))
    ).toEqual({
      title: 'Model quota exceeded',
      message: 'You exceeded your current quota, please check your plan and billing details.',
      details: QUOTA_ERROR,
    });
  });

  it('renders unknown future errors with a safe generic fallback', () => {
    render(
      <ChatTurnError
        error={error({
          providerKind: 'brand_new_rejection',
          message: 'A future provider-specific rejection',
        })}
      />
    );

    const alert = screen.getByRole('alert');
    expect(alert).toHaveTextContent('Model request failed');
    expect(alert).toHaveTextContent('A future provider-specific rejection');
  });

  it('keeps long technical details wrapped in a collapsed expandable region', () => {
    const details = `provider_failure: ${'very-long-provider-payload'.repeat(20)}`;
    render(
      <ChatTurnError
        error={error({
          providerKind: 'unknown',
          technicalDetails: details,
        })}
      />
    );

    const disclosure = screen.getByText('Technical details').closest('details');
    expect(disclosure).not.toHaveAttribute('open');
    fireEvent.click(screen.getByText('Technical details'));
    expect(disclosure).toHaveAttribute('open');

    const detailBody = screen.getByText(details);
    expect(detailBody).toHaveClass('break-words');
    expect(detailBody).toHaveClass('[overflow-wrap:anywhere]');
  });

  it('labels a transient backend-unreachable failure distinctly from a provider failure', () => {
    const presentation = presentChatTurnError(
      error({
        message: 'Failed to fetch',
        code: 'session_load_unreachable',
        scope: 'transport',
        retryable: true,
        technicalDetails: 'Failed to fetch',
      })
    );

    // Backend (biorouterd) down reads as its own thing — not "check provider
    // settings", which would send the user hunting for a key that is fine.
    expect(presentation.title).toBe('Backend unreachable');
    expect(presentation.message).toContain('backend is restarting');
    expect(presentation.message).not.toContain('provider settings');
  });

  it('labels a dropped stream as a mid-response interruption', () => {
    const presentation = presentChatTurnError(
      error({
        message: 'stream closed',
        code: 'stream_interrupted',
        scope: 'transport',
        retryable: true,
      })
    );
    expect(presentation.title).toBe('Connection dropped');
    expect(presentation.message).toContain('dropped before the response finished');
  });

  it('renders a Retry action for a retryable error and invokes onRetry once', () => {
    const onRetry = vi.fn();
    render(
      <ChatTurnError
        error={error({ code: 'submit_error', scope: 'transport', retryable: true })}
        onRetry={onRetry}
      />
    );

    const retry = screen.getByTestId('chat-turn-error-retry');
    fireEvent.click(retry);
    expect(onRetry).toHaveBeenCalledTimes(1);
  });

  it('omits the Retry action when the error is not retryable', () => {
    render(<ChatTurnError error={error({ retryable: false })} onRetry={vi.fn()} />);
    expect(screen.queryByTestId('chat-turn-error-retry')).toBeNull();
  });

  it('omits the Retry action when no handler is provided', () => {
    render(<ChatTurnError error={error({ retryable: true })} />);
    expect(screen.queryByTestId('chat-turn-error-retry')).toBeNull();
  });

  it('uses connection-specific recovery copy without dropping technical details', () => {
    const presentation = presentChatTurnError(
      error({
        message: 'Failed to fetch',
        scope: 'transport',
        technicalDetails: 'Submit error: Failed to fetch',
      })
    );

    expect(presentation).toMatchObject({
      title: 'Model connection failed',
      message: expect.stringContaining('Check your connection and provider settings'),
      details: 'Submit error: Failed to fetch',
    });
  });

  /**
   * M2 — the daemon synthesizes `stream_ended_without_terminal` for any turn
   * whose writers produced no ending, INCLUDING one the user stopped. Read as
   * an ordinary internal failure it says "Model turn ended unexpectedly", which
   * blames the model for an ending the user asked for. The store re-codes the
   * two stop-shaped endings; the presentation has to name them.
   */
  it('names a user-requested ending instead of blaming the model', () => {
    const presentation = presentChatTurnError(
      error({
        message: 'You stopped this turn, so it ended without a result.',
        code: 'turn_stopped_by_user',
        scope: 'internal',
        retryable: false,
        technicalDetails: 'The stream for this turn ended without a result. Please retry.',
      })
    );

    expect(presentation.title).toBe('Turn stopped');
    expect(presentation.title).not.toBe('Model turn ended unexpectedly');
    expect(presentation.message).toContain('You stopped this turn');
  });

  it('names a stop the backend never confirmed', () => {
    const presentation = presentChatTurnError(
      error({
        message:
          'Biorouter could not stop this turn. It may still be running on the backend. HTTP 504',
        code: 'stop_not_confirmed',
        scope: 'internal',
        retryable: false,
      })
    );

    expect(presentation.title).toBe('Stop not confirmed');
    expect(presentation.message).toContain('may still be running');
  });

  it('still blames nothing but the model for an ordinary internal failure', () => {
    expect(
      presentChatTurnError(
        error({
          message: 'The stream for this turn ended without a result. Please retry.',
          code: 'stream_ended_without_terminal',
          scope: 'internal',
          retryable: true,
        })
      ).title
    ).toBe('Model turn ended unexpectedly');
  });

  it('does not duplicate a backend error message that is already in the transcript', () => {
    const turnError = error({ message: 'Authentication failed. Status: 401 Unauthorized' });
    const messages = [
      {
        role: 'assistant',
        content: [
          {
            type: 'text',
            text: 'Ran into this error: Authentication failed. Status: 401 Unauthorized.',
          },
        ],
        metadata: { userVisible: true, agentVisible: true },
      },
    ] as Message[];

    expect(hasVisibleTurnErrorMessage(turnError, messages)).toBe(true);
    expect(hasVisibleTurnErrorMessage(error({ message: 'Failed to fetch' }), messages)).toBe(false);
  });
});
