import './decisionsChat.css';
import { ENTITY_ICONS } from '../icons/entity-icons';
import { Button } from '../ui/button';
import { TranscriptRow } from '../TranscriptRow';
import type { ArtifactSource } from '../artifacts/artifactTypes';
import {
  decisionHistoryArtifact,
  type DecisionTurnMarker as DecisionTurnMarkerData,
} from '../../utils/sessionDecisions';
import { markerItemText } from './decisionWire';

export const DECISION_MARKER_COPY = {
  lead: 'Decision models',
  turnHistory: 'Open this turn in the decision history',
  itemHistory: 'Decision history',
  itemHistoryLabel: (line: string) => `Open ${line} in the decision history`,
  fired: 'A monitor fired this turn',
};

/**
 * The marker's one line: "Decision models: monitor Form reached 18 checks, last P
 * 0.31 · decide 600 chunks, 41 passed".
 */
export function decisionMarkerLine(marker: DecisionTurnMarkerData): string {
  return `${DECISION_MARKER_COPY.lead}: ${marker.items.map(markerItemText).join(' · ')}`;
}

export interface DecisionTurnMarkerProps {
  sessionId: string;
  marker: DecisionTurnMarkerData;
  onOpenArtifact: (artifact: ArtifactSource) => void;
}

/**
 * Every decision call of one turn, as ONE transcript line (owner addendum A1):
 * per source, how many calls, how many passed, and for a monitor the last
 * probability it saw. It opens into one line per source, each linking to the
 * decision history filtered to that source in this turn, and a link to the whole
 * turn. A turn's calls are never listed here one by one; the history is the only
 * place that does.
 */
export function DecisionTurnMarker({ sessionId, marker, onOpenArtifact }: DecisionTurnMarkerProps) {
  const fired = marker.items.some((item) => item.fired);
  return (
    <TranscriptRow
      icon={ENTITY_ICONS.decision}
      glyph="decision"
      label={decisionMarkerLine(marker)}
      data-testid="decision-turn-marker"
      className="br-decision-marker"
      statusLabel={fired ? DECISION_MARKER_COPY.fired : undefined}
    >
      <ul className="br-decision-marker__items">
        {marker.items.map((item) => {
          const line = markerItemText(item);
          return (
            <li
              key={`${item.origin}:${item.originId ?? ''}`}
              className="br-decision-marker__item"
              data-origin={item.origin}
              data-fired={item.fired ? '' : undefined}
              data-testid="decision-turn-marker-item"
            >
              <span className="br-decision-marker__text">{line}</span>
              <Button
                type="button"
                variant="link"
                size="xs"
                aria-label={DECISION_MARKER_COPY.itemHistoryLabel(line)}
                onClick={() =>
                  onOpenArtifact(
                    decisionHistoryArtifact(sessionId, {
                      turnId: marker.turnId,
                      origin: item.origin,
                      ...(item.originId ? { originId: item.originId } : {}),
                    })
                  )
                }
              >
                {DECISION_MARKER_COPY.itemHistory}
              </Button>
            </li>
          );
        })}
      </ul>
      <Button
        type="button"
        variant="link"
        size="xs"
        className="br-decision-marker__turn"
        onClick={() =>
          onOpenArtifact(decisionHistoryArtifact(sessionId, { turnId: marker.turnId }))
        }
      >
        {DECISION_MARKER_COPY.turnHistory}
      </Button>
    </TranscriptRow>
  );
}
