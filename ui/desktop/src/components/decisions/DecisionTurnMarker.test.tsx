import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { DecisionTurnMarker, DECISION_MARKER_COPY, decisionMarkerLine } from './DecisionTurnMarker';
import {
  parseDecisionTurnMarker,
  type DecisionTurnMarker as Marker,
} from '../../utils/sessionDecisions';

const monitorItem = {
  origin: 'monitor',
  originId: 'form',
  label: 'Form reached',
  count: 18,
  passed: 0,
  failed: 18,
  errors: 0,
  lastPassProbability: 0.31,
  fired: false,
};

const decideItem = {
  origin: 'tool',
  originId: 'call-9',
  label: 'decide',
  count: 600,
  passed: 41,
  failed: 559,
  errors: 0,
  lastPassProbability: 0.12,
  fired: false,
};

const gateItem = {
  origin: 'gate',
  originId: 'probe',
  label: 'Probe write test',
  count: 2,
  passed: 0,
  failed: 2,
  errors: 0,
  lastPassProbability: 0.03,
  fired: false,
};

const marker = parseDecisionTurnMarker({ turnId: 'turn-3', items: [monitorItem, decideItem] })!;

function show(data: Marker = marker) {
  const onOpenArtifact = vi.fn();
  const view = render(
    <DecisionTurnMarker sessionId="chat-1" marker={data} onOpenArtifact={onOpenArtifact} />
  );
  return { ...view, onOpenArtifact };
}

function toggle() {
  return screen.getByRole('button', { name: decisionMarkerLine(marker) });
}

describe('the per-turn decision marker', () => {
  it('is one short line for the whole turn: counts, no lead, no names, no probability', () => {
    show();
    expect(decisionMarkerLine(marker)).toBe(
      'Monitor checked 18 times, quiet · 1 decide call read 600 chunks'
    );
    const rows = screen.getAllByTestId('decision-turn-marker');
    expect(rows).toHaveLength(1);
    const label = within(rows[0]).getByTestId('decision-turn-marker-label');
    expect(label).not.toHaveTextContent('Decision models');
    expect(label).not.toHaveTextContent('Form reached');
    expect(label).not.toHaveTextContent('0.31');
    // Each part is its own span, so a part that does not fit drops whole.
    expect(
      within(label)
        .getAllByTestId('decision-turn-marker-part')
        .map((part) => part.textContent)
    ).toEqual(['Monitor checked 18 times, quiet', '· 1 decide call read 600 chunks']);
    // Closed, it lists nothing: 618 calls are one line.
    expect(screen.queryAllByTestId('decision-turn-marker-item')).toHaveLength(0);
  });

  it('puts the most serious part first and folds the rest into +N more', () => {
    const three = parseDecisionTurnMarker({
      turnId: 'turn-3',
      items: [
        monitorItem,
        decideItem,
        gateItem,
        { ...decideItem, origin: 'chat', originId: null, count: 3 },
      ],
    })!;
    show(three);
    const parts = screen
      .getAllByTestId('decision-turn-marker-part')
      .map((part) => part.textContent);
    expect(parts).toEqual([
      'Gate checked twice, not passed',
      '· Monitor checked 18 times, quiet',
      '+2 more',
    ]);
  });

  it('opens into one line per source, each a link to that source in this turn', async () => {
    const { onOpenArtifact } = show();
    fireEvent.click(toggle());
    const items = screen.getAllByTestId('decision-turn-marker-item');
    expect(items.map((item) => item.dataset.origin)).toEqual(['monitor', 'tool']);
    const line = within(items[0]).getByRole('button', {
      name: DECISION_MARKER_COPY.itemHistoryLabel('Form reached'),
    });
    expect(line).toHaveTextContent('Form reached: checked 18 times, quiet');
    expect(line).toHaveClass('br-decision-link', 'br-decision-link--line');
    expect(within(items[1]).getByRole('button')).toHaveTextContent('Decide call: 600 chunks');
    fireEvent.click(line);
    expect(onOpenArtifact).toHaveBeenLastCalledWith(
      expect.objectContaining({
        kind: 'decisions',
        sessionId: 'chat-1',
        filter: { turnId: 'turn-3', origin: 'monitor', originId: 'form' },
      })
    );
    // Its tooltip says what it opens, in the same words as its name.
    await userEvent.hover(line);
    expect(
      (await screen.findAllByText(DECISION_MARKER_COPY.itemHistoryLabel('Form reached'))).length
    ).toBeGreaterThan(0);
  });

  it('ends with one link to the whole turn', () => {
    const { onOpenArtifact } = show();
    fireEvent.click(toggle());
    const turn = screen.getByRole('button', { name: DECISION_MARKER_COPY.turnHistory });
    expect(turn).toHaveTextContent(/^Decision history$/);
    fireEvent.click(turn);
    expect(onOpenArtifact).toHaveBeenLastCalledWith(
      expect.objectContaining({ kind: 'decisions', filter: { turnId: 'turn-3' } })
    );
  });

  it('states an error, then a gate not passed, then a fired monitor', () => {
    const status = (items: Array<Record<string, unknown>>) => {
      const data = parseDecisionTurnMarker({ turnId: 't', items })!;
      const { unmount } = render(
        <DecisionTurnMarker sessionId="chat-1" marker={data} onOpenArtifact={vi.fn()} />
      );
      const labels = Object.values(DECISION_MARKER_COPY.status).filter(
        (words) => screen.queryByLabelText(words) !== null
      );
      unmount();
      return labels;
    };
    const fired = { ...monitorItem, passed: 1, failed: 17, fired: true };
    expect(status([fired])).toEqual([DECISION_MARKER_COPY.status.fired]);
    expect(status([fired, gateItem])).toEqual([DECISION_MARKER_COPY.status.gate]);
    expect(status([fired, gateItem, { ...decideItem, errors: 2 }])).toEqual([
      DECISION_MARKER_COPY.status.error,
    ]);
    expect(status([monitorItem])).toEqual([]);
  });

  it('says when a monitor fired', () => {
    const fired = parseDecisionTurnMarker({
      turnId: 'turn-3',
      items: [{ ...monitorItem, passed: 2, failed: 16, fired: true }],
    })!;
    show(fired);
    expect(screen.getByLabelText(DECISION_MARKER_COPY.status.fired)).toBeInTheDocument();
    expect(screen.getByTestId('decision-turn-marker')).toHaveTextContent(
      'Monitor checked 18 times, fired twice'
    );
  });
});

describe('opening the marker at the end of the conversation (G-7)', () => {
  it('scrolls the open well into view once the panel has opened', async () => {
    const scrollIntoView = vi.fn();
    const original = Element.prototype.scrollIntoView;
    Element.prototype.scrollIntoView = scrollIntoView;
    try {
      show();
      fireEvent.click(toggle());
      const well = screen.getByTestId('decision-turn-marker-well');
      await waitFor(() => expect(scrollIntoView).toHaveBeenCalledTimes(1), { timeout: 2000 });
      expect(scrollIntoView.mock.contexts[0]).toBe(well);
      expect(scrollIntoView.mock.calls[0][0]).toMatchObject({ block: 'nearest' });
    } finally {
      Element.prototype.scrollIntoView = original;
    }
  });
});
