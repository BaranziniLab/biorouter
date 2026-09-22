import { beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen } from '@testing-library/react';
import UserMessage from './UserMessage';
import type { Message } from '../api';
import { labelledRefTag, refTag } from '../utils/resourceRefs';

// Issue #65 — the transcript's half of the reference-tag ruling.
//
// A sent message keeps the tag: it is what the agent read, what a reload
// replays and what an edit re-sends. So the transcript has to draw it as a chip
// too, or the user watches their own message come back as XML.

const userMessage = (text: string): Message => ({
  id: 'message-1',
  role: 'user',
  created: 1,
  content: [{ type: 'text', text }],
  metadata: { userVisible: true, agentVisible: true },
});

beforeEach(() => {
  Object.assign(window, { electron: { logInfo: vi.fn() } });
});

describe('a sent message shows its references as chips', () => {
  it('draws the chip and never the markup', () => {
    render(<UserMessage message={userMessage(`please run ${refTag('skill', 'my skill')}`)} />);

    expect(screen.getByTestId('resource-ref-chip-name')).toHaveTextContent('my skill');
    expect(document.body.textContent).toContain('please run');
    expect(document.body.textContent).not.toContain('biorouter-ref');
  });

  it('reads a knowledge base by the name the user picked', () => {
    render(
      <UserMessage message={userMessage(labelledRefTag('knowledge_base', 'soul', 'Soul & Body'))} />
    );

    expect(screen.getByTestId('resource-ref-chip-name')).toHaveTextContent('Soul & Body');
  });

  it('offers no remove control on a message already sent', () => {
    render(<UserMessage message={userMessage(refTag('skill', 'my skill'))} />);

    expect(screen.queryByRole('button', { name: /^remove/i })).not.toBeInTheDocument();
  });

  it('leaves a message with no reference exactly as it was', () => {
    render(<UserMessage message={userMessage('just a message')} />);

    expect(screen.getByText('just a message')).toBeInTheDocument();
    expect(screen.queryByTestId('resource-ref-chip')).not.toBeInTheDocument();
  });

  it('leaves a tag it cannot parse as visible text', () => {
    const broken = `<biorouter-ref type="skill" name="never closed`;
    render(<UserMessage message={userMessage(broken)} />);

    expect(document.body.textContent).toContain(broken);
    expect(screen.queryByTestId('resource-ref-chip')).not.toBeInTheDocument();
  });
});

describe('editing a sent message', () => {
  // The edit box is a second composer, and the same rule applies: prose in the
  // textarea, references as chips. Without this, "Edit" is the one place the
  // markup still leaks — and a user tidying their sentence would delete half a
  // tag and silently lose the reference.
  it('keeps the markup out of the edit box and the reference on screen', () => {
    render(<UserMessage message={userMessage(`hello ${refTag('skill', 'my skill')}`)} />);

    fireEvent.click(screen.getByRole('button', { name: /edit message:/i }));

    const box = screen.getByRole('textbox', {
      name: 'Edit message content',
    }) as HTMLTextAreaElement;
    expect(box.value).toBe('hello');
    expect(screen.getByTestId('resource-ref-chip-name')).toHaveTextContent('my skill');
  });

  it('re-sends the edited prose with the reference still attached', () => {
    const onMessageUpdate = vi.fn();
    render(
      <UserMessage
        message={userMessage(`hello ${refTag('skill', 'my skill')}`)}
        onMessageUpdate={onMessageUpdate}
      />
    );

    fireEvent.click(screen.getByRole('button', { name: /edit message:/i }));
    fireEvent.change(screen.getByRole('textbox', { name: 'Edit message content' }), {
      target: { value: 'hello there' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Diverge with the edited message' }));

    expect(onMessageUpdate).toHaveBeenCalledWith(
      'message-1',
      `hello there ${refTag('skill', 'my skill')}`,
      'diverge'
    );
  });

  it('lets the user drop a reference while editing', () => {
    const onMessageUpdate = vi.fn();
    render(
      <UserMessage
        message={userMessage(`hello ${refTag('skill', 'my skill')}`)}
        onMessageUpdate={onMessageUpdate}
      />
    );

    fireEvent.click(screen.getByRole('button', { name: /edit message:/i }));
    fireEvent.click(screen.getByRole('button', { name: /^remove/i }));
    fireEvent.click(screen.getByRole('button', { name: 'Diverge with the edited message' }));

    expect(onMessageUpdate).toHaveBeenCalledWith('message-1', 'hello', 'diverge');
  });
});
