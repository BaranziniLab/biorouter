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

// `/bug` is drawn as a chip in the composer, so the message it sent draws the
// same chip. The text keeps `/bug` at its start (the daemon reads it there), so
// the edit box has to give it back on save, chip or not.
describe('a sent /bug message', () => {
  it('draws the command chip first, then the prose', () => {
    render(<UserMessage message={userMessage('/bug the chart panel is blank')} />);

    const chip = screen.getByTestId('command-chip');
    expect(chip).toHaveAttribute('data-command', 'bug');
    expect(screen.getByTestId('command-chip-name')).toHaveTextContent('Report a bug');
    expect(document.body.textContent).toContain('the chart panel is blank');
    expect(screen.queryByText(/^\/bug/)).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /^remove/i })).not.toBeInTheDocument();
  });

  it('draws a bare /bug, sent with no prose, as the chip alone', () => {
    render(<UserMessage message={userMessage('/bug')} />);

    expect(screen.getByTestId('command-chip')).toBeInTheDocument();
  });

  it('draws the chip beside a reference chip', () => {
    render(<UserMessage message={userMessage(`/bug blank ${refTag('skill', 'my skill')}`)} />);

    expect(screen.getByTestId('command-chip')).toBeInTheDocument();
    expect(screen.getByTestId('resource-ref-chip-name')).toHaveTextContent('my skill');
    expect(document.body.textContent).not.toContain('biorouter-ref');
  });

  it('leaves /bugs and a mid-sentence /bug as text', () => {
    render(<UserMessage message={userMessage('/bugs are fixed, see /bug later')} />);

    expect(screen.queryByTestId('command-chip')).not.toBeInTheDocument();
    expect(screen.getByText('/bugs are fixed, see /bug later')).toBeInTheDocument();
  });

  it('edits the prose and keeps /bug at the start of what it re-sends', () => {
    const onMessageUpdate = vi.fn();
    render(
      <UserMessage
        message={userMessage(`/bug blank ${refTag('skill', 'my skill')}`)}
        onMessageUpdate={onMessageUpdate}
      />
    );

    fireEvent.click(screen.getByRole('button', { name: /edit message:/i }));
    const box = screen.getByRole('textbox', {
      name: 'Edit message content',
    }) as HTMLTextAreaElement;
    expect(box.value).toBe('blank');
    expect(screen.getByTestId('command-chip')).toBeInTheDocument();

    fireEvent.change(box, { target: { value: 'the chart is blank' } });
    fireEvent.click(screen.getByRole('button', { name: 'Diverge with the edited message' }));

    expect(onMessageUpdate).toHaveBeenCalledWith(
      'message-1',
      `/bug the chart is blank ${refTag('skill', 'my skill')}`,
      'diverge'
    );
  });

  it('shows a chip-only message as the chip in the edit box, and saving it unchanged is a no-op', () => {
    const onMessageUpdate = vi.fn();
    render(<UserMessage message={userMessage('/bug')} onMessageUpdate={onMessageUpdate} />);

    fireEvent.click(screen.getByRole('button', { name: /edit message:/i }));
    expect(
      (screen.getByRole('textbox', { name: 'Edit message content' }) as HTMLTextAreaElement).value
    ).toBe('');
    expect(screen.getByTestId('command-chip')).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Diverge with the edited message' }));
    expect(onMessageUpdate).not.toHaveBeenCalled();
  });

  it('drops the command when its chip is removed in the edit box', () => {
    const onMessageUpdate = vi.fn();
    render(
      <UserMessage message={userMessage('/bug keep this')} onMessageUpdate={onMessageUpdate} />
    );

    fireEvent.click(screen.getByRole('button', { name: /edit message:/i }));
    fireEvent.click(screen.getByRole('button', { name: 'Remove command /bug' }));
    fireEvent.click(screen.getByRole('button', { name: 'Diverge with the edited message' }));

    expect(onMessageUpdate).toHaveBeenCalledWith('message-1', 'keep this', 'diverge');
  });
});
