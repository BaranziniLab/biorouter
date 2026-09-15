import { beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen } from '@testing-library/react';
import UserMessage from './UserMessage';
import type { Message } from '../api';

/**
 * D4 (fix/steer-always-lands): a steer the daemon accepted and then stored
 * UNANSWERED — the turn ended (a Stop, a spend cap, a provider abort) before
 * the model read it — used to render exactly like a delivered message, so the
 * person saw their words sitting in the middle of the conversation with no
 * reply and no way to tell they never landed.
 */
const steer = (steerOutcome?: 'unanswered'): Message => ({
  id: 'steer-1',
  role: 'user',
  created: 1,
  content: [{ type: 'text', text: 'use the 2024 cohort' }],
  metadata: { userVisible: true, agentVisible: true, ...(steerOutcome ? { steerOutcome } : {}) },
});

beforeEach(() => {
  Object.assign(window, { electron: { logInfo: vi.fn() } });
});

describe('an unanswered steer', () => {
  it('says it was not answered and offers to send it again', () => {
    const onSendAgain = vi.fn();
    render(<UserMessage message={steer('unanswered')} onSendAgain={onSendAgain} />);

    expect(screen.getByTestId('steer-unanswered')).toHaveTextContent(
      'Not answered: the turn ended before the agent read this.'
    );
    fireEvent.click(screen.getByRole('button', { name: 'Send this message again' }));
    expect(onSendAgain).toHaveBeenCalledWith('use the 2024 cohort');
  });

  it('shows no Send again while a turn is running (no callback)', () => {
    render(<UserMessage message={steer('unanswered')} />);
    expect(screen.getByTestId('steer-unanswered')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Send this message again' })).toBeNull();
  });

  it('keeps unconfirmed text visible and offers an explicit retry', () => {
    const onSendAgain = vi.fn();
    render(<UserMessage message={steer()} deliveryUnconfirmed onSendAgain={onSendAgain} />);
    expect(screen.getByText('use the 2024 cohort')).toBeVisible();
    expect(
      screen.getByText('Delivery unconfirmed. Check the transcript before sending again.')
    ).toBeVisible();
    fireEvent.click(screen.getByRole('button', { name: 'Send this message again' }));
    expect(onSendAgain).toHaveBeenCalledWith('use the 2024 cohort');
  });

  it('draws a delivered steer as an ordinary message', () => {
    render(<UserMessage message={steer()} onSendAgain={vi.fn()} />);
    expect(screen.queryByTestId('steer-unanswered')).toBeNull();
  });
});
