import { readFileSync } from 'node:fs';
import path from 'node:path';
import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { ChatTurnStopped } from './ChatTurnStopped';

/** vitest runs with `ui/desktop` as its root — the idiom `BaseChat.privacy.test.tsx` uses. */
const read = (...p: string[]) => readFileSync(path.join(process.cwd(), ...p), 'utf8');

/**
 * F5 (QA of 7c96d796, 2026-09-10): a Stop that worked stated no outcome. The
 * store decides WHEN the line shows (`chatStreamStore.test.ts`, "a Stop the
 * daemon confirms"); this file pins what it says and where it goes.
 */
describe('ChatTurnStopped', () => {
  it('states the outcome in words, in a polite live region', () => {
    render(<ChatTurnStopped />);
    expect(screen.getByRole('status')).toHaveTextContent('Stopped.');
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
   * Item 7: the daemon stores the notice and the transcript draws it, so the
   * tail line must never sit beside that row as a second "Stopped.". Asserted at
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
    expect(screen.getByRole('status')).toHaveTextContent('Stopped.');
  });
});
