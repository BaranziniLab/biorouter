import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { DecisionTurnMarker, DECISION_MARKER_COPY, decisionMarkerLine } from './DecisionTurnMarker';
import { parseDecisionTurnMarker } from '../../utils/sessionDecisions';

const marker = parseDecisionTurnMarker({
  turnId: 'turn-3',
  items: [
    {
      origin: 'monitor',
      originId: 'form',
      label: 'Form reached',
      count: 18,
      passed: 0,
      failed: 18,
      errors: 0,
      lastPassProbability: 0.31,
      fired: false,
    },
    {
      origin: 'tool',
      originId: 'call-9',
      label: 'decide',
      count: 600,
      passed: 41,
      failed: 559,
      errors: 0,
      lastPassProbability: 0.12,
      fired: false,
    },
  ],
})!;

function show() {
  const onOpenArtifact = vi.fn();
  const view = render(
    <DecisionTurnMarker sessionId="chat-1" marker={marker} onOpenArtifact={onOpenArtifact} />
  );
  return { ...view, onOpenArtifact };
}

describe('the per-turn decision marker', () => {
  it('is one line for the whole turn, however many calls it made', () => {
    show();
    expect(decisionMarkerLine(marker)).toBe(
      'Decision models: monitor Form reached 18 checks, last P 0.31 · decide 600 chunks, 41 passed'
    );
    const rows = screen.getAllByTestId('decision-turn-marker');
    expect(rows).toHaveLength(1);
    expect(rows[0]).toHaveTextContent(decisionMarkerLine(marker));
    // Closed, it lists nothing: 618 calls are one line.
    expect(screen.queryAllByTestId('decision-turn-marker-item')).toHaveLength(0);
  });

  it('opens into one line per source, never one per call', () => {
    show();
    fireEvent.click(screen.getByRole('button', { name: /Decision models/ }));
    const items = screen.getAllByTestId('decision-turn-marker-item');
    expect(items.map((item) => item.dataset.origin)).toEqual(['monitor', 'tool']);
  });

  it('links each source, and the whole turn, to the decision history filtered to it', () => {
    const { onOpenArtifact } = show();
    fireEvent.click(screen.getByRole('button', { name: /Decision models/ }));
    fireEvent.click(
      screen.getByRole('button', {
        name: DECISION_MARKER_COPY.itemHistoryLabel('monitor Form reached 18 checks, last P 0.31'),
      })
    );
    expect(onOpenArtifact).toHaveBeenLastCalledWith(
      expect.objectContaining({
        kind: 'decisions',
        sessionId: 'chat-1',
        filter: { turnId: 'turn-3', origin: 'monitor', originId: 'form' },
      })
    );
    fireEvent.click(screen.getByRole('button', { name: DECISION_MARKER_COPY.turnHistory }));
    expect(onOpenArtifact).toHaveBeenLastCalledWith(
      expect.objectContaining({ kind: 'decisions', filter: { turnId: 'turn-3' } })
    );
  });

  it('says when a monitor fired', () => {
    const fired = { ...marker, items: [{ ...marker.items[0], fired: true }] };
    render(<DecisionTurnMarker sessionId="chat-1" marker={fired} onOpenArtifact={vi.fn()} />);
    expect(screen.getByLabelText(DECISION_MARKER_COPY.fired)).toBeInTheDocument();
    expect(screen.getByTestId('decision-turn-marker')).toHaveTextContent('fired');
  });
});
