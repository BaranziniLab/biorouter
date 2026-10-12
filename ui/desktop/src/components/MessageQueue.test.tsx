import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import type { ComponentProps } from 'react';
import { describe, expect, it, vi } from 'vitest';
import { MessageQueue, canSteerMessage } from './MessageQueue';
import { refTag } from '../utils/resourceRefs';
import { getSteerAriaKeyShortcuts } from '../utils/keyboardShortcuts';
import { COMPOSER_COPY } from './composer/copy';

const COPY = COMPOSER_COPY.queue;

const queuedMessage = {
  id: 'message-1',
  content: 'and add better comments',
  timestamp: Date.now(),
};

const renderQueue = (overrides: Partial<ComponentProps<typeof MessageQueue>> = {}) => {
  const onSteerMessage = vi.fn();
  const onStopAndSend = vi.fn();

  render(
    <MessageQueue
      queuedMessages={[queuedMessage]}
      onRemoveMessage={vi.fn()}
      onClearQueue={vi.fn()}
      onSteerMessage={onSteerMessage}
      onStopAndSend={onStopAndSend}
      {...overrides}
    />
  );

  return { onSteerMessage, onStopAndSend };
};

describe('MessageQueue actions', () => {
  it('distinguishes adding to the current turn from stopping and sending', async () => {
    const user = userEvent.setup();
    const { onSteerMessage, onStopAndSend } = renderQueue();

    const addNow = screen.getByRole('button', { name: COPY.addNowLabel });
    const stopAndSend = screen.getByRole('button', { name: COPY.stopAndSendLabel });

    // Icon buttons: the name carries the words, the tooltip repeats them.
    expect(addNow.querySelector('.lucide-message-square-plus')).not.toBeNull();
    expect(stopAndSend.querySelector('.lucide-arrow-up')).not.toBeNull();
    expect(addNow).not.toHaveAttribute('title');
    expect(stopAndSend).not.toHaveAttribute('title');

    await user.click(addNow);
    await user.click(stopAndSend);

    expect(onSteerMessage).toHaveBeenCalledWith(queuedMessage.id);
    expect(onStopAndSend).toHaveBeenCalledWith(queuedMessage.id);
  });

  it("reads as Codex's queue: a count, then the message after an arrow", () => {
    renderQueue();
    expect(screen.getByText(`${COPY.header} · 1`)).toBeInTheDocument();
    expect(screen.getByText('and add better comments')).toBeInTheDocument();
  });

  it('uses the same explicit actions in the expanded queue', async () => {
    const user = userEvent.setup();
    renderQueue();

    await user.click(screen.getByRole('button', { name: '1 message queued. Expand queue.' }));

    expect(screen.getByRole('button', { name: COPY.addNowLabel })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: COPY.stopAndSendLabel })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: COPY.remove })).toBeInTheDocument();
  });

  it('keeps attachment messages out of the text-only add-now path', () => {
    renderQueue({
      queuedMessages: [
        {
          ...queuedMessage,
          attachments: [{ path: '/tmp/image.png', kind: 'image' }],
        },
      ],
    });

    expect(screen.queryByRole('button', { name: COPY.addNowLabel })).toBeNull();
    expect(screen.getByRole('button', { name: COPY.stopAndSendLabel })).toBeInTheDocument();
    expect(screen.getByText(/1 attachment/)).toBeInTheDocument();
  });

  it('uses no off-scale sizes and no native titles', () => {
    renderQueue();
    expect(document.body.innerHTML).not.toMatch(/text-\[1[01]px\]/);
    expect(document.querySelector('[title]')).toBeNull();
  });
});

/**
 * `canSteerMessage` is exported because `ChatInput`'s Cmd/Ctrl+Enter fallback
 * steers the front of this queue. The button and the chord have to ask ONE
 * question; a shortcut that re-derived eligibility would drift from the button
 * it mirrors the moment either side gained a rule.
 */
describe('MessageQueue steer eligibility', () => {
  it('is text-only, and says so to any caller', () => {
    expect(canSteerMessage({})).toBe(true);
    expect(canSteerMessage({ attachments: [] })).toBe(true);
    expect(canSteerMessage({ attachments: [{ path: '/tmp/image.png', kind: 'image' }] })).toBe(
      false
    );
  });
});

/**
 * The chord steers the FRONT of the queue and only the front, so it is
 * announced on that row alone. Naming it on row two would name a key that does
 * something else: it would take row one.
 */
describe('MessageQueue shortcut hint', () => {
  const second = { id: 'message-2', content: 'and rerun the fit', timestamp: Date.now() };

  it('announces the chord on the next message, in the collapsed bar', () => {
    renderQueue({ queuedMessages: [queuedMessage, second] });

    expect(screen.getByRole('button', { name: COPY.addNowLabel })).toHaveAttribute(
      'aria-keyshortcuts',
      getSteerAriaKeyShortcuts()
    );
  });

  it('announces it on the first expanded row and no other', async () => {
    const user = userEvent.setup();
    renderQueue({ queuedMessages: [queuedMessage, second] });

    await user.click(screen.getByRole('button', { name: /queued\. Expand queue\./i }));
    const [first, rest] = screen.getAllByRole('button', { name: COPY.addNowLabel });

    expect(first).toHaveAttribute('aria-keyshortcuts', getSteerAriaKeyShortcuts());
    expect(rest).not.toHaveAttribute('aria-keyshortcuts');
  });
});

// Issue #65 — the queue renders inside the composer, so the same rule applies:
// the user never sees `<biorouter-ref …>` markup, and the reference survives an
// edit. A queued message is one the composer already built, tags and all.
describe('MessageQueue references', () => {
  const withRef = {
    id: 'message-1',
    content: `and add better comments ${refTag('skill', 'my skill')}`,
    timestamp: Date.now(),
  };

  it('shows a chip in the queued row instead of the markup', () => {
    renderQueue({ queuedMessages: [withRef] });

    expect(screen.getByTestId('resource-ref-chip-name')).toHaveTextContent('my skill');
    expect(document.body.textContent).not.toContain('biorouter-ref');
  });

  it('keeps the markup out of the inline editor and the reference on the message', async () => {
    const user = userEvent.setup();
    const onEditMessage = vi.fn();
    renderQueue({ queuedMessages: [withRef], onEditMessage });

    await user.click(screen.getByRole('button', { name: /queued\. Expand queue\./i }));
    await user.click(screen.getByText('and add better comments'));
    const box = screen.getByRole('textbox') as HTMLTextAreaElement;
    expect(box.value).toBe('and add better comments');

    await user.clear(box);
    await user.type(box, 'and tidy up');
    await user.click(screen.getByRole('button', { name: /^save$/i }));

    expect(onEditMessage).toHaveBeenCalledWith(
      'message-1',
      `and tidy up ${refTag('skill', 'my skill')}`
    );
  });
});
