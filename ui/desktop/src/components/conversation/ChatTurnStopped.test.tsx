import { readFileSync } from 'node:fs';
import path from 'node:path';
import { render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import type { Message } from '../../api';
import ProgressiveMessageList from '../ProgressiveMessageList';
import { ChatTurnStopped } from './ChatTurnStopped';

/** vitest runs with `ui/desktop` as its root — the idiom `BaseChat.privacy.test.tsx` uses. */
const read = (...p: string[]) => readFileSync(path.join(process.cwd(), ...p), 'utf8');

/**
 * F5 (QA of 7c96d796, 2026-09-10): a Stop that worked stated no outcome. The
 * store decides WHEN the line shows (`chatStreamStore.test.ts`, "a Stop the
 * daemon confirms"); this file pins what it says and where it goes.
 */
describe('ChatTurnStopped', () => {
  // The owner's cancel note (2026-11-17) is what a confirmed Stop says live too,
  // so the live line and the stored row read the same.
  it('states the outcome in words, in a polite live region', () => {
    render(<ChatTurnStopped />);
    expect(screen.getByRole('status')).toHaveTextContent(
      'The person canceled this turn before it finished.'
    );
  });

  // The success half of M2's notice is quiet: nothing about this ending is an
  // error, and nothing is left for the user to do about it.
  it('is neither an alert nor the error card', () => {
    render(<ChatTurnStopped />);
    expect(screen.queryByRole('alert')).toBeNull();
    expect(screen.queryByTestId('chat-turn-error')).toBeNull();
  });

  /**
   * BaseChat cannot be mounted in jsdom (see `BaseChat.privacy.test.tsx`), so
   * its half is asserted at the source: the line is fed by the store's
   * `stopConfirmed`, and it sits in the slot a failed Stop's notice takes —
   * the transcript's tail, after the pending tool calls, beside `ChatTurnError`.
   */
  it('is rendered by BaseChat from the store, in the failed Stop notice’s slot', () => {
    const source = read('src', 'components', 'BaseChat.tsx');

    expect(source).toMatch(/const \{[^}]*\bstopConfirmed,[^}]*\} = useChatStream\(/);
    const tail = /<PendingToolCallList\b[\s\S]*?<\/SearchView>/.exec(source);
    expect(tail, 'BaseChat no longer ends its transcript with PendingToolCallList').not.toBeNull();
    expect(tail![0]).toMatch(
      /<ChatTurnError\b[\s\S]*\{stopConfirmed && !transcriptEndsStopped\(messages\) && \(\s*<ChatTurnStopped \/>\s*\)\}/
    );
  });

  /**
   * Item 7: the daemon stores the row and the transcript draws it, so the tail
   * line must never sit beside that row as a second stopped line. Asserted at
   * the source for the same reason as the case above, and the predicate's own
   * behaviour below.
   */
  it('is suppressed when the transcript already ends on the stored notice', async () => {
    const { transcriptEndsStopped } = await import('./turnStoppedNotice');
    const notice = {
      id: 'n',
      role: 'assistant',
      created: 1,
      content: [{ type: 'systemNotification', notificationType: 'inlineMessage', msg: 'Stopped.' }],
      metadata: { userVisible: true, agentVisible: false },
    } as const;
    const reply = {
      id: 'r',
      role: 'assistant',
      created: 1,
      content: [{ type: 'text', text: 'The telescope' }],
      metadata: { userVisible: true, agentVisible: true },
    } as const;
    expect(transcriptEndsStopped([reply, notice] as never)).toBe(true);
    expect(transcriptEndsStopped([notice, reply] as never)).toBe(false);
    expect(transcriptEndsStopped([])).toBe(false);
  });

  it('takes the transcript row’s spacing when drawn as a stored notice', () => {
    const { container } = render(<ChatTurnStopped inTranscript />);
    const root = container.querySelector('[data-testid="chat-turn-stopped"]');
    expect(root?.className).not.toMatch(/\bmt-4\b/);
    expect(screen.getByRole('status')).toHaveTextContent(
      'The person canceled this turn before it finished.'
    );
  });
});

/**
 * The owner's cancel note (2026-11-17): a turn the person cancelled ends on a
 * stored note that the model reads too, so the daemon stores it as the agent's
 * text, not as a notice every wire format drops. The transcript must still draw
 * it as the quiet stopped line, in its own words, never as a reply bubble with
 * copy actions; and a reload, another window and History read the same row.
 */
describe('the stored cancel note', () => {
  const NOTE = 'The person canceled this turn before it finished.';
  const question: Message = {
    id: 'q',
    role: 'user',
    created: 1,
    content: [{ type: 'text', text: 'When was the telescope invented?' }],
    metadata: { userVisible: true, agentVisible: true },
  } as Message;
  const halfReply: Message = {
    id: 'r',
    role: 'assistant',
    created: 2,
    content: [{ type: 'text', text: 'The telescope was invented in' }],
    metadata: { userVisible: true, agentVisible: true },
  } as Message;
  const note: Message = {
    id: 'note',
    role: 'assistant',
    created: 3,
    content: [{ type: 'text', text: NOTE }],
    metadata: { userVisible: true, agentVisible: true },
  } as Message;

  it('a_stored_cancel_note_is_drawn_as_the_stopped_line', async () => {
    const { TURN_STOPPED_NOTICE, isTurnStoppedNotice, transcriptEndsStopped } =
      await import('./turnStoppedNotice');
    expect(TURN_STOPPED_NOTICE).toBe(NOTE);
    expect(isTurnStoppedNotice(note)).toBe(true);
    expect(transcriptEndsStopped([question, halfReply, note])).toBe(true);

    render(
      <ProgressiveMessageList
        messages={[question, halfReply, note]}
        onRunInTerminal={null}
        chat={{ sessionId: 'stored-cancel-note' }}
        toolCallNotifications={new Map()}
        isUserMessage={(m: Message) => m.role !== 'assistant'}
        batchSize={15}
        batchDelay={30}
        showLoadingThreshold={30}
        onOpenArtifact={vi.fn()}
      />
    );

    const line = screen.getByTestId('chat-turn-stopped');
    expect(line).toHaveTextContent(NOTE);
    expect(screen.getAllByText(NOTE)).toHaveLength(1);
    expect(screen.getByText('The telescope was invented in')).toBeInTheDocument();
    // The line carries no reply actions: it is the stopped line, not an answer.
    expect(line.closest('[data-testid="message-container"]')?.querySelector('button')).toBeNull();
  });

  it('keeps drawing a chat stored before the note, which ended on the short line', async () => {
    const { isTurnStoppedNotice } = await import('./turnStoppedNotice');
    const older = {
      id: 'older',
      role: 'assistant',
      created: 3,
      content: [{ type: 'systemNotification', notificationType: 'inlineMessage', msg: 'Stopped.' }],
      metadata: { userVisible: true, agentVisible: false },
    } as Message;
    expect(isTurnStoppedNotice(older)).toBe(true);
    render(<ChatTurnStopped inTranscript label="Stopped." />);
    expect(screen.getByRole('status')).toHaveTextContent('Stopped.');
  });

  // After the prose a stopped reply had streamed, the daemon stores the note as
  // a new paragraph, because the model reads the two rows as one reply.
  it('draws the note stored as a new paragraph in the same words', async () => {
    const { isTurnStoppedNotice, turnStoppedNoticeText } = await import('./turnStoppedNotice');
    const paragraph = {
      ...note,
      content: [{ type: 'text', text: `\n\n${NOTE}` }],
    } as Message;
    expect(isTurnStoppedNotice(paragraph)).toBe(true);
    expect(turnStoppedNoticeText(paragraph)).toBe(NOTE);
  });

  it('is not mistaken for a reply that merely says the same thing at more length', async () => {
    const { isTurnStoppedNotice } = await import('./turnStoppedNotice');
    const reply = {
      ...note,
      content: [{ type: 'text', text: `${NOTE} Here is what I had so far.` }],
    } as Message;
    const userEcho = { ...note, role: 'user' } as Message;
    expect(isTurnStoppedNotice(reply)).toBe(false);
    expect(isTurnStoppedNotice(userEcho)).toBe(false);
  });
});
