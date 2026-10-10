import { act, cleanup, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ChatSummaryRail, type ChatSummaryRailProps } from './ChatSummaryRail';
import { summaryCopy } from './copy';

/**
 * The rail's card keeps three promises that no other test holds: it mounts
 * entered when the rail appears because of geometry, it plays its entrance once
 * when the person opened it, and it reports its exit so the grid can drop the
 * column afterwards (`hooks/useSummaryRail.ts`).
 */

const base: ChatSummaryRailProps = {
  id: 'summary-rail',
  'data-preview-area': 'rail',
  still: true,
  state: 'open',
  onExited: vi.fn(),
  toolCalls: '6',
  billedTokens: '211k',
  artifacts: '1',
  codeDelta: { added: 0, removed: 0 },
  hasWorkflow: false,
  onWorkflow: vi.fn(),
  onDiagnostics: vi.fn(),
  todos: { items: [], loading: false, error: false, refresh: vi.fn() },
};

const card = (container: HTMLElement) => container.querySelector<HTMLElement>('.br-summary-card')!;

function fire(element: Element, type: 'animationend' | 'animationcancel') {
  act(() => {
    element.dispatchEvent(new Event(type, { bubbles: true }));
  });
}

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe('the rail’s landmark', () => {
  it('is an aside named Chat summary, in the grid’s rail cell, drawing the summary', () => {
    render(<ChatSummaryRail {...base} />);
    const rail = screen.getByRole('complementary', { name: summaryCopy.title });
    expect(rail).toHaveAttribute('id', 'summary-rail');
    expect(rail).toHaveAttribute('data-preview-area', 'rail');
    expect(rail).toContainElement(screen.getByText(summaryCopy.toolCalls));
  });
});

describe('the card’s entrance', () => {
  it('mounts entered, and still, when the rail appears because of geometry', () => {
    const { container } = render(<ChatSummaryRail {...base} still />);
    expect(card(container)).toHaveAttribute('data-entered');
    expect(screen.getByRole('complementary')).toHaveAttribute('data-rail-still');
  });

  it('plays once when the person opened it, then latches', () => {
    const { container, rerender } = render(<ChatSummaryRail {...base} still={false} />);
    expect(card(container)).not.toHaveAttribute('data-entered');
    expect(screen.getByRole('complementary')).not.toHaveAttribute('data-rail-still');

    fire(card(container), 'animationend');
    expect(card(container)).toHaveAttribute('data-entered');
    expect(base.onExited).not.toHaveBeenCalled();

    // Read at mount only: a later change of the prop never replays it.
    rerender(<ChatSummaryRail {...base} still={false} />);
    expect(card(container)).toHaveAttribute('data-entered');
  });

  it('counts a cancelled animation as played, so a resize does not leave it to replay', () => {
    const { container } = render(<ChatSummaryRail {...base} still={false} />);
    fire(card(container), 'animationcancel');
    expect(card(container)).toHaveAttribute('data-entered');
  });

  it('ignores the animations of what is inside the card', () => {
    const { container } = render(<ChatSummaryRail {...base} still={false} />);
    fire(screen.getByText(summaryCopy.toolCalls), 'animationend');
    expect(card(container)).not.toHaveAttribute('data-entered');
  });
});

describe('the card’s exit', () => {
  it('reports once its exit animation has run, and not before', () => {
    const { container, rerender } = render(<ChatSummaryRail {...base} />);
    expect(card(container)).toHaveAttribute('data-state', 'open');
    rerender(<ChatSummaryRail {...base} state="closed" />);
    expect(card(container)).toHaveAttribute('data-state', 'closed');
    expect(base.onExited).not.toHaveBeenCalled();

    fire(card(container), 'animationend');
    expect(base.onExited).toHaveBeenCalledTimes(1);
  });

  it('reports an exit that a resize cancelled', () => {
    const { container, rerender } = render(<ChatSummaryRail {...base} />);
    rerender(<ChatSummaryRail {...base} state="closed" />);
    fire(card(container), 'animationcancel');
    expect(base.onExited).toHaveBeenCalledTimes(1);
  });

  it('slides back in when it is reopened during its exit', () => {
    const { container, rerender } = render(<ChatSummaryRail {...base} still />);
    expect(card(container)).toHaveAttribute('data-entered');
    rerender(<ChatSummaryRail {...base} state="closed" />);
    expect(card(container)).not.toHaveAttribute('data-entered');
    rerender(<ChatSummaryRail {...base} state="open" />);
    expect(card(container)).not.toHaveAttribute('data-entered');
    fire(card(container), 'animationend');
    expect(card(container)).toHaveAttribute('data-entered');
    expect(base.onExited).not.toHaveBeenCalled();
  });
});
