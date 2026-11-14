import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import type { Message } from '../../api';
import { SystemNotificationInline } from './SystemNotificationInline';

function inlineNotice(msg: string): Message {
  return {
    id: 'notice',
    role: 'assistant',
    created: 1,
    content: [{ type: 'systemNotification', notificationType: 'inlineMessage', msg }],
    metadata: { userVisible: true, agentVisible: false },
  } as Message;
}

/**
 * Item 7: a stopped turn now ends on a STORED notice, so a reload, another
 * window and History show the interruption. It must read as the same quiet line
 * a confirmed Stop draws live (`ChatTurnStopped`), not as a second, differently
 * styled sentence — the transcript row IS that line now.
 */
describe('SystemNotificationInline', () => {
  it('draws a stored stop notice as the “Stopped.” line', () => {
    render(<SystemNotificationInline message={inlineNotice('Stopped.')} />);
    expect(screen.getByTestId('chat-turn-stopped')).toHaveTextContent('Stopped.');
  });

  it('leaves every other inline notice as it was', () => {
    render(<SystemNotificationInline message={inlineNotice('Compaction complete')} />);
    expect(screen.queryByTestId('chat-turn-stopped')).toBeNull();
    expect(screen.getByText('Compaction complete')).toBeInTheDocument();
  });

  // A notice that merely mentions stopping is not the stop marker.
  it('does not mistake a sentence about stopping for the marker', () => {
    render(<SystemNotificationInline message={inlineNotice('Stopped. Retrying in 5s')} />);
    expect(screen.queryByTestId('chat-turn-stopped')).toBeNull();
  });

  // A usage-limit notice is the same quiet line, marked with its kind.
  it('marks a stored usage-limit notice with its kind', () => {
    const text = 'Usage limit reached for this chat: 1,000 of 1,000 tokens.';
    render(<SystemNotificationInline message={inlineNotice(text)} />);
    expect(screen.getByText(text)).toHaveAttribute('data-usage-limit-notice', 'chatReached');
  });

  // Owner, 2026-11-13: a notice past one line keeps its first sentence (the reason) on the
  // row, and the guidance after it is the info icon's.
  it('keeps the reason on the row and folds the guidance behind the info icon', () => {
    const reason = 'Monthly usage limit reached: $12.00 of $10.00.';
    const guidance =
      'To continue, raise or lift it in Settings > App > Usage. The month resets on the 1st, local time.';
    render(<SystemNotificationInline message={inlineNotice(`${reason} ${guidance}`)} />);
    expect(screen.getByText(reason)).toHaveAttribute('data-usage-limit-notice', 'monthReached');
    expect(screen.getByRole('button', { name: 'About this notice' })).toHaveAccessibleDescription(
      guidance
    );
  });

  it('folds a tail too long for the info icon under Details, word for word', () => {
    const reason =
      'Nothing ran: this reply is a call to shell written as text, not a tool call Biorouter can run.';
    const tail =
      'Smaller local models often do this. A larger model, or fewer extensions turned on in this chat, may help. Try again after changing either.';
    render(<SystemNotificationInline message={inlineNotice(`${reason} ${tail}`)} />);
    expect(screen.getByText(reason)).toBeInTheDocument();
    expect(screen.queryByText(tail)).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Details' }));
    expect(screen.getByText(tail)).toBeInTheDocument();
  });

  it('leaves the marker off any other notice', () => {
    render(<SystemNotificationInline message={inlineNotice('Compaction complete')} />);
    expect(screen.getByText('Compaction complete')).not.toHaveAttribute('data-usage-limit-notice');
  });
});
