import { render, screen } from '@testing-library/react';
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
});
