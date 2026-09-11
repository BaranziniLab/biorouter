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
    expect(tail![0]).toMatch(/<ChatTurnError\b[\s\S]*\{stopConfirmed && <ChatTurnStopped \/>\}/);
  });
});
