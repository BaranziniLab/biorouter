import { render } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import SharedSessionView from './SharedSessionView';
import type { SharedSessionDetails } from '../../sharedSessions';

// The transcript body itself is exercised by `SessionViewComponents.*.test.tsx`;
// what is under test here is the SHELL — the reading column and the two page
// headers that have to line up with it — so the body is stubbed rather than
// dragging MarkdownContent and ToolCallWithResponse in behind it.
vi.mock('./SessionViewComponents', () => ({
  SessionMessages: () => <div data-testid="messages" />,
}));

vi.mock('../artifacts/ArtifactViewer', () => ({
  default: () => <div data-testid="artifact-viewer" />,
}));

function shared(over: Partial<SharedSessionDetails> = {}): SharedSessionDetails {
  return {
    share_token: 'token-1',
    created_at: 1757200000,
    base_url: 'https://example.invalid',
    description: 'Shared cohort walkthrough',
    working_dir: '/Users/wgu/Desktop/BioRouter',
    message_count: 2,
    total_tokens: 4096,
    messages: [],
    ...over,
  };
}

function renderView(session: SharedSessionDetails | null = shared()) {
  return render(
    <SharedSessionView session={session} isLoading={false} error={null} onRetry={vi.fn()} />
  );
}

/**
 * The shared transcript is the one view that LEAVES the machine, and until
 * 2026-09-07 it was the one with no reading measure at all: a bare `px-8` flex
 * column, so the same conversation was drawn pane-wide here and in a column
 * everywhere else. It reads the chat measure now, like its local twin
 * `SessionHistoryView` and like the live chat.
 *
 * ⚠ jsdom sees the ATTRIBUTE and the class STRING, never a width — no layout
 * engine, and Tailwind never runs, so `max-w-measure-chat` computes to nothing
 * here. The 760px column was measured in a browser. The source-level half of
 * the rule (no `<ReadableContent` left on the default size, no second
 * `max-w-*`) is in `styles/measures.test.ts`.
 */
describe('SharedSessionView sits on the chat measure', () => {
  it('renders one reading column, at the chat size', () => {
    const { container } = renderView();

    const columns = [...container.querySelectorAll('.biorouter-readable-content')];
    expect(columns).toHaveLength(1);
    expect((columns[0] as HTMLElement).dataset.size).toBe('chat');
  });

  /**
   * The header hairlines are FULL-BLEED across the reading column: each one
   * cancels the column's inset with a negative margin and puts it back as
   * padding. The two halves must be the same number or the rule stops short of
   * the column's edge (too small) or runs past it (too large) — a defect that
   * is invisible in jsdom as a width and perfectly visible as a class string,
   * which is why it is asserted this way round.
   */
  it('insets the full-bleed page headers by exactly the column’s own padding', () => {
    const { container } = renderView();

    const column = container.querySelector('.biorouter-readable-content') as HTMLElement;
    expect(column.className).toContain('px-6');

    const headers = [...container.querySelectorAll('.biorouter-page-header')];
    expect(headers.length).toBeGreaterThan(0);
    for (const header of headers) {
      expect(header.className).toContain('-mx-6');
      expect(header.className).toContain('px-6');
    }
  });

  /** The 896px replay fork never existed here, and must not arrive. */
  it('declares no second measure inside the column', () => {
    const { container } = renderView();

    for (const element of container.querySelectorAll<HTMLElement>('*')) {
      expect(element.className.toString()).not.toMatch(/\bmax-w-(?:3xl|4xl|5xl|6xl|7xl)\b/);
    }
  });

  /**
   * A shared link that has not resolved yet still renders the shell. The column
   * is part of the shell, not of the loaded conversation, so a reader watching
   * the page settle does not see the content jump from pane-wide to columned.
   */
  it('keeps the column while the shared chat is still loading', () => {
    const { container } = render(
      <SharedSessionView session={null} isLoading={true} error={null} onRetry={vi.fn()} />
    );

    const columns = [...container.querySelectorAll('.biorouter-readable-content')];
    expect(columns).toHaveLength(1);
    expect((columns[0] as HTMLElement).dataset.size).toBe('chat');
  });
});
