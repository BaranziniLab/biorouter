import { beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen } from '@testing-library/react';
import UserMessage from './UserMessage';
import type { Message } from '../api';

const message: Message = {
  id: 'message-1',
  role: 'user',
  created: 1,
  content: [{ type: 'text', text: 'original prompt' }],
  metadata: { userVisible: true, agentVisible: true },
};

beforeEach(() => {
  Object.assign(window, {
    electron: {
      logInfo: vi.fn(),
    },
  });
});

describe('UserMessage edit actions', () => {
  it('intrinsically sizes only the bubble while preserving the message width cap', () => {
    render(<UserMessage message={message} onMessageUpdate={vi.fn()} />);

    const bubble = screen.getByTestId('user-message-bubble');
    expect(bubble).toHaveClass('w-fit', 'max-w-full', 'min-w-0', 'self-end');
    expect(bubble.firstElementChild).toHaveClass('min-w-0', 'break-words');
    expect(bubble.parentElement).toHaveClass('flex-col', 'min-w-0');
    expect(bubble.parentElement).not.toHaveClass('items-end');
    expect(bubble.parentElement?.parentElement).toHaveClass('max-w-[85%]', 'w-fit', 'min-w-0');
    expect(bubble.closest('.message')?.querySelector('[data-message-meta="end"]')).not.toBeNull();
  });

  it('uses Diverge copy and emits the diverge action', () => {
    const onMessageUpdate = vi.fn();
    render(<UserMessage message={message} onMessageUpdate={onMessageUpdate} />);

    fireEvent.click(screen.getByRole('button', { name: /edit message:/i }));

    expect(screen.getAllByText('Diverge')).toHaveLength(2);
    expect(screen.queryByText(/Fork/)).not.toBeInTheDocument();
    expect(screen.queryByText(/Branch/)).not.toBeInTheDocument();

    fireEvent.change(screen.getByRole('textbox', { name: 'Edit message content' }), {
      target: { value: 'updated prompt' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Diverge with the edited message' }));

    expect(onMessageUpdate).toHaveBeenCalledWith('message-1', 'updated prompt', 'diverge');
  });

  it('labels a message another agent injected (BR-71 §5)', () => {
    render(
      <UserMessage
        message={{
          ...message,
          id: 'message-2',
          metadata: {
            userVisible: true,
            agentVisible: true,
            provenance: {
              kind: 'agent_injection',
              fromSessionId: 's-parent',
              fromSessionName: 'Planning chat',
            },
          },
        }}
        onMessageUpdate={vi.fn()}
      />
    );
    expect(screen.getByText(/injected by Planning chat/i)).toBeTruthy();
  });

  it('keeps the injection label up while the message is being edited', () => {
    // §5 calls the label permanent, which is why the chip is mounted above the
    // isEditing branch rather than inside the bubble it replaces. Without this
    // case that placement is defended by prose alone.
    render(
      <UserMessage
        message={{
          ...message,
          id: 'message-3',
          metadata: {
            userVisible: true,
            agentVisible: true,
            provenance: { kind: 'agent_injection', fromSessionName: 'Planning chat' },
          },
        }}
        onMessageUpdate={vi.fn()}
      />
    );

    fireEvent.click(screen.getByRole('button', { name: /edit message:/i }));

    expect(screen.getByRole('textbox', { name: 'Edit message content' })).toBeTruthy();
    expect(screen.getByText(/injected by Planning chat/i)).toBeTruthy();
  });

  it('shows no chip on an ordinary same-session message', () => {
    render(<UserMessage message={message} onMessageUpdate={vi.fn()} />);
    expect(document.querySelector('[data-provenance-kind]')).toBeNull();
  });
});
