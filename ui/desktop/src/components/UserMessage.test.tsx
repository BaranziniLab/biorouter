import { beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen } from '@testing-library/react';
import UserMessage from './UserMessage';
import type { Message } from '../api';
import { transcriptCopy } from './transcript/copy';

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

  it('edits in one card: Cancel, Save as new chat, and Save, each saying what it does', () => {
    const onMessageUpdate = vi.fn();
    render(<UserMessage message={message} onMessageUpdate={onMessageUpdate} />);

    fireEvent.click(screen.getByRole('button', { name: /edit message:/i }));

    // No line explaining the buttons any more: their tooltips do.
    expect(screen.queryByText(/updates this chat/)).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Cancel editing' })).toBeInTheDocument();
    expect(screen.queryByText(/Fork/)).not.toBeInTheDocument();
    expect(screen.queryByText(/Branch/)).not.toBeInTheDocument();

    fireEvent.change(screen.getByRole('textbox', { name: 'Edit message content' }), {
      target: { value: 'updated prompt' },
    });
    fireEvent.click(screen.getByRole('button', { name: transcriptCopy.edit.saveAsNew }));
    expect(onMessageUpdate).toHaveBeenCalledWith('message-1', 'updated prompt', 'diverge');
  });

  // R-27: the editor is the composer's card, so it is the subtle fill with a
  // transparent edge (the coral focus edge turns it), never a hairline box, and
  // the secondary button inside steps back to the page ground.
  it('edits on the filled composer card, with no resting outline', () => {
    render(<UserMessage message={message} onMessageUpdate={vi.fn()} />);
    fireEvent.click(screen.getByRole('button', { name: /edit message:/i }));

    const card = screen
      .getByRole('textbox', { name: 'Edit message content' })
      .closest('.biorouter-composer-card');
    expect(card).not.toBeNull();
    expect(card).toHaveClass(
      'bg-background-fill',
      'border',
      'border-transparent',
      'br-fill-surface'
    );
    expect(card?.className).not.toMatch(/border-border-|bg-background-default/);
  });

  it('hands focus back to the Edit button when the editor closes', () => {
    // The editor replaces the bubble and the button that opened it, so a
    // keyboard user would otherwise be dropped on the page.
    render(<UserMessage message={message} onMessageUpdate={vi.fn()} />);
    const open = () => fireEvent.click(screen.getByRole('button', { name: /edit message:/i }));

    open();
    fireEvent.keyDown(screen.getByRole('textbox', { name: 'Edit message content' }), {
      key: 'Escape',
    });
    expect(screen.getByRole('button', { name: /edit message:/i })).toHaveFocus();

    open();
    fireEvent.click(screen.getByRole('button', { name: 'Cancel editing' }));
    expect(screen.getByRole('button', { name: /edit message:/i })).toHaveFocus();
  });

  it('saves in place from the filled button and from Cmd+Enter', () => {
    const onMessageUpdate = vi.fn();
    const { rerender } = render(
      <UserMessage message={message} onMessageUpdate={onMessageUpdate} />
    );

    fireEvent.click(screen.getByRole('button', { name: /edit message:/i }));
    fireEvent.change(screen.getByRole('textbox', { name: 'Edit message content' }), {
      target: { value: 'updated prompt' },
    });
    fireEvent.click(screen.getByRole('button', { name: transcriptCopy.edit.save }));
    expect(onMessageUpdate).toHaveBeenLastCalledWith('message-1', 'updated prompt', 'edit');

    rerender(
      <UserMessage message={{ ...message, id: 'message-9' }} onMessageUpdate={onMessageUpdate} />
    );
    fireEvent.click(screen.getByRole('button', { name: /edit message:/i }));
    const box = screen.getByRole('textbox', { name: 'Edit message content' });
    fireEvent.change(box, { target: { value: 'again' } });
    fireEvent.keyDown(box, { key: 'Enter', metaKey: true });
    expect(onMessageUpdate).toHaveBeenLastCalledWith('message-9', 'again', 'edit');
  });

  it('floats its actions over the bubble instead of reserving a row under it', () => {
    render(<UserMessage message={message} onMessageUpdate={vi.fn()} />);

    const cluster = document.querySelector('[data-message-meta="end"]')!;
    expect(cluster).toHaveClass('br-message-actions');
    // Anchored to the bubble's column, so its right edge is the bubble's.
    expect(cluster.parentElement).toHaveClass('br-message-actions-anchor');
    // Two glyph buttons, named; their words are tooltips, not text.
    const edit = screen.getByRole('button', { name: /edit message:/i });
    const copy = screen.getByRole('button', { name: transcriptCopy.message.copyMessage });
    for (const button of [edit, copy]) {
      expect(cluster).toContainElement(button);
      expect(button.textContent?.trim()).toBe('');
    }
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
    // The actions float from the bubble's column, as usual.
    const cluster = document.querySelector('[data-message-meta="end"]');
    expect(cluster?.parentElement).toHaveClass('br-message-actions-anchor');
  });

  it("puts the actions on the chip's line, before the chip, so a hover never covers it", () => {
    // Vision V2-12: the cluster floats at the bubble's top right, which is
    // where a provenance chip sits ("Spawn context" heads every subagent chat).
    // Floating over it hid the chip and its tooltip on every hover.
    render(
      <UserMessage
        message={{
          ...message,
          id: 'message-4',
          metadata: {
            userVisible: true,
            agentVisible: true,
            provenance: { kind: 'spawn_context', fromSessionId: 's-parent' },
          },
        }}
        onMessageUpdate={vi.fn()}
      />
    );
    const chip = document.querySelector('[data-provenance-kind="spawn_context"]') as HTMLElement;
    const cluster = document.querySelector('[data-message-meta="end"]')!;
    expect(cluster.parentElement).toHaveClass('br-message-actions-chip');
    expect(cluster.parentElement).toContainElement(chip);
    expect(cluster.compareDocumentPosition(chip) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    // One cluster, not two.
    expect(document.querySelectorAll('[data-message-meta]')).toHaveLength(1);
  });
});
