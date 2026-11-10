import { Fragment, useRef } from 'react';
import './decisionsChat.css';
import './decisionsPanel.css';
import { ENTITY_ICONS } from '../icons/entity-icons';
import { TranscriptRow } from '../TranscriptRow';
import type { ArtifactSource } from '../artifacts/artifactTypes';
import {
  decisionHistoryArtifact,
  type DecisionTurnMarker as DecisionTurnMarkerData,
} from '../../utils/sessionDecisions';
import { DecisionHistoryLink } from './DecisionHistoryLink';
import { useRevealWhenOpened } from './useRevealWhenOpened';
import {
  MARKER_PARTS_SHOWN,
  markerItemsByImportance,
  markerParts,
  markerSourceLine,
  markerStatus,
} from './decisionWire';

export const DECISION_MARKER_COPY = {
  turnHistory: 'Open this turn in the decision history',
  itemHistoryLabel: (name: string) => `Open ${name} in the decision history`,
  more: (n: number) => `+${n} more`,
  status: {
    error: 'A decision call failed this turn',
    gate: 'A phase gate did not pass this turn',
    fired: 'A monitor fired this turn',
  },
};

/** The source a line opens, by name: the part of its line before the colon. */
function sourceName(line: string): string {
  const colon = line.indexOf(':');
  return colon > 0 ? line.slice(0, colon) : line;
}

/**
 * The closed line's parts in importance order: at most two, then `+N more`
 * (F-19). Each is its own span, so a part that does not fit drops whole and a
 * number is never cut.
 */
export function decisionMarkerParts(marker: DecisionTurnMarkerData): string[] {
  const parts = markerParts(marker).map((part) => part.text);
  if (parts.length <= MARKER_PARTS_SHOWN) return parts;
  return [
    ...parts.slice(0, MARKER_PARTS_SHOWN),
    DECISION_MARKER_COPY.more(parts.length - MARKER_PARTS_SHOWN),
  ];
}

/** The closed line as one string (its accessible name): `A · B +1 more`. */
export function decisionMarkerLine(marker: DecisionTurnMarkerData): string {
  const parts = decisionMarkerParts(marker);
  const shown = markerParts(marker).length;
  const more = shown > MARKER_PARTS_SHOWN ? parts.pop() : undefined;
  return [parts.join(' · '), more].filter(Boolean).join(' ');
}

export interface DecisionTurnMarkerProps {
  sessionId: string;
  marker: DecisionTurnMarkerData;
  onOpenArtifact: (artifact: ArtifactSource) => void;
}

/**
 * The open marker: one line per source in importance order, each itself a link, then
 * the turn's link. Mounted only while open, so it scrolls itself into view once the
 * panel has opened (G-7: a marker at the end of the conversation opened under the
 * composer bar).
 */
function MarkerWell({ sessionId, marker, onOpenArtifact }: DecisionTurnMarkerProps) {
  const ref = useRef<HTMLDivElement | null>(null);
  useRevealWhenOpened(ref);
  return (
    <div ref={ref} className="br-decision-marker__well" data-testid="decision-turn-marker-well">
      <ul className="br-decision-marker__items">
        {markerItemsByImportance(marker).map((item) => {
          const line = markerSourceLine(item);
          return (
            <li
              key={`${item.origin}:${item.originId ?? ''}`}
              className="br-decision-marker__item"
              data-origin={item.origin}
              data-fired={item.fired ? '' : undefined}
              data-testid="decision-turn-marker-item"
            >
              <DecisionHistoryLink
                label={DECISION_MARKER_COPY.itemHistoryLabel(sourceName(line))}
                onOpen={() =>
                  onOpenArtifact(
                    decisionHistoryArtifact(sessionId, {
                      turnId: marker.turnId,
                      origin: item.origin,
                      ...(item.originId ? { originId: item.originId } : {}),
                    })
                  )
                }
              >
                {line}
              </DecisionHistoryLink>
            </li>
          );
        })}
      </ul>
      <DecisionHistoryLink
        label={DECISION_MARKER_COPY.turnHistory}
        testId="decision-turn-marker-turn-link"
        onOpen={() => onOpenArtifact(decisionHistoryArtifact(sessionId, { turnId: marker.turnId }))}
      />
    </div>
  );
}

/**
 * Every decision call of one turn, as ONE transcript line (owner addendum A1): a
 * short, fixed summary, one part per kind of source, counts first and the most
 * serious part first ("Gate checked twice, not passed · Monitors checked 12 times,
 * fired twice"). It opens into one line per source, each itself a link to the
 * decision history filtered to that source in this turn, then a link to the whole
 * turn. A turn's calls are never listed here one by one; the history is the only
 * place that does.
 */
export function DecisionTurnMarker({ sessionId, marker, onOpenArtifact }: DecisionTurnMarkerProps) {
  const status = markerStatus(marker);
  const parts = decisionMarkerParts(marker);
  return (
    <TranscriptRow
      icon={ENTITY_ICONS.decision}
      glyph="decision"
      label={
        <span className="br-decision-marker__label" data-testid="decision-turn-marker-label">
          {parts.map((part, index) => (
            // The space between parts sits OUTSIDE them: the only place the line may
            // break, and the word gap a screen reader hears.
            <Fragment key={`${index}:${part}`}>
              {index > 0 ? ' ' : null}
              <span className="br-decision-marker__part" data-testid="decision-turn-marker-part">
                {index === 0 || index >= MARKER_PARTS_SHOWN ? part : `· ${part}`}
              </span>
            </Fragment>
          ))}
        </span>
      }
      data-testid="decision-turn-marker"
      className="br-decision-marker"
      statusLabel={status ? DECISION_MARKER_COPY.status[status] : undefined}
    >
      <MarkerWell sessionId={sessionId} marker={marker} onOpenArtifact={onOpenArtifact} />
    </TranscriptRow>
  );
}
