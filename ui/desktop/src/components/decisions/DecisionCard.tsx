import './decisionsChat.css';
import * as React from 'react';
import { AlertTriangle, Check, Lock, X } from '../icons/app-icons';
import { ENTITY_ICONS } from '../icons/entity-icons';
import { Button } from '../ui/button';
import { Progress } from '../ui/progress';
import type { ArtifactSource } from '../artifacts/artifactTypes';
import {
  answerKey,
  answerProbability,
  calibrationText,
  decisionHistoryArtifact,
  decisionModelLabel,
  formatLatency,
  formatProbability,
  outcomeLabel,
  readableAnswer,
  thresholdOnOption,
  thresholdSummary,
  type Decision,
  type DecisionRecord,
  type NamedQuestion,
} from '../../utils/sessionDecisions';
import { orderedProbabilities } from './decisionWire';
import { useDecisionRecord } from './decisionChatApi';

const DecisionMark = ENTITY_ICONS.decision;

export const DECISION_CARD_COPY = {
  region: (question: string) => `Decision: ${question}`,
  probabilities: 'Probabilities',
  passed: 'Passed',
  failed: 'Below threshold',
  threshold: (summary: string) => `Threshold: ${summary}`,
  got: (p: string) => `, got ${p}`,
  history: 'Decision history',
  historyLabel: 'Open this decision in the decision history',
  privateModel: 'Private model',
  publicModel: 'Public model',
  notDecided: 'Not decided',
  refused: 'Refused',
};

/**
 * One answered question: the question, the answer large, one bar per option
 * (yes and no for a yes or no question) with the threshold drawn on the bar it is
 * compared against, and whether it passed. Status is said in words and by shape,
 * never by colour alone. An uncalibrated source says so in words.
 */
function DecisionBlock({
  decision,
  question,
}: {
  decision: Decision;
  question: NamedQuestion | undefined;
}) {
  const bars = orderedProbabilities(decision, question);
  const answerP = answerProbability(decision);
  const chosen = answerKey(decision.answer);
  const threshold = question?.threshold ?? null;
  const prompt = question?.instructions.trim() || decision.question;
  return (
    <div className="br-decision-card__decision" data-question={decision.question}>
      <p className="br-decision-card__question">{prompt}</p>
      <div className="br-decision-card__headline">
        <span className="br-decision-card__answer" data-testid="decision-answer">
          {readableAnswer(decision, question)}
        </span>
        {answerP !== null && (
          <span className="br-decision-card__answer-p">P {formatProbability(answerP)}</span>
        )}
        {decision.passed !== null && (
          <span
            className="br-decision-card__verdict"
            data-passed={decision.passed ? 'true' : 'false'}
            data-testid="decision-verdict"
          >
            {decision.passed ? <Check aria-hidden /> : <X aria-hidden />}
            {decision.passed ? DECISION_CARD_COPY.passed : DECISION_CARD_COPY.failed}
          </span>
        )}
      </div>
      <ul className="br-decision-bars" aria-label={DECISION_CARD_COPY.probabilities}>
        {bars.map(({ key, p }) => {
          const label = outcomeLabel(question, key);
          const mark = thresholdOnOption(threshold, key);
          return (
            <li
              key={key}
              className="br-decision-bars__row"
              data-key={key}
              data-answer={key === chosen ? '' : undefined}
              data-testid="decision-bar"
            >
              <span className="br-decision-bars__label">{label}</span>
              <span className="br-decision-bars__track">
                <Progress
                  value={p * 100}
                  label={`${label} ${formatProbability(p)}`}
                  fillProps={{ className: 'br-decision-bars__fill' }}
                />
                {mark !== null && (
                  <span
                    className="br-decision-bars__threshold"
                    style={{ left: `${mark * 100}%` }}
                    data-testid="decision-threshold-mark"
                    aria-hidden
                  />
                )}
              </span>
              <span className="br-decision-bars__value">{formatProbability(p)}</span>
            </li>
          );
        })}
      </ul>
      {threshold && (
        <p className="br-decision-card__line" data-testid="decision-threshold">
          {DECISION_CARD_COPY.threshold(thresholdSummary(threshold, question))}
          {threshold.type === 'at_least_level' && decision.passProbability !== null
            ? DECISION_CARD_COPY.got(formatProbability(decision.passProbability))
            : ''}
        </p>
      )}
      {!decision.calibrated && (
        <p className="br-decision-card__line" data-testid="decision-calibration">
          {calibrationText(decision.source, decision.calibrated)}
        </p>
      )}
    </div>
  );
}

export interface DecisionCardViewProps {
  record: DecisionRecord;
  /** The chat the row belongs to, for the history link. */
  sessionId: string;
  onOpenArtifact: (artifact: ArtifactSource) => void;
}

/** A decision chat's reply as a card, drawn from its decision-log row. */
export function DecisionCardView({ record, sessionId, onOpenArtifact }: DecisionCardViewProps) {
  const questions = new Map(record.questions.map((question) => [question.name, question]));
  const first = record.decisions[0];
  const regionQuestion =
    (first && (questions.get(first.question)?.instructions.trim() || first.question)) ||
    record.questions[0]?.instructions ||
    record.label ||
    record.id;
  // A Baseten model reads as its alias (joey-1) here as everywhere else; the id
  // Baseten says served it stays in the history's "Served by".
  const ref = record.model ?? first?.model ?? null;
  const model =
    ref?.provider === 'baseten_decision'
      ? decisionModelLabel(ref)
      : (record.servedModel ??
        decisionModelLabel(record.model) ??
        decisionModelLabel(first?.model));
  const tier = record.tier ?? first?.tier ?? null;
  const latency = formatLatency(record.latencyMs ?? first?.latencyMs);
  const failed = record.outcome === 'error' || record.outcome === 'refused';
  return (
    <section
      className="br-decision-card br-fill-surface"
      aria-label={DECISION_CARD_COPY.region(regionQuestion)}
      data-testid="decision-card"
      data-outcome={record.outcome}
    >
      {record.decisions.map((decision) => (
        <DecisionBlock
          key={decision.question}
          decision={decision}
          question={questions.get(decision.question)}
        />
      ))}
      {failed && (
        <p className="br-decision-card__error" data-testid="decision-error">
          <AlertTriangle aria-hidden />
          <span>
            {record.outcome === 'refused'
              ? DECISION_CARD_COPY.refused
              : DECISION_CARD_COPY.notDecided}
            {record.error ? `: ${record.error}` : ''}
          </span>
        </p>
      )}
      <div className="br-decision-card__meta" data-testid="decision-meta">
        {model && (
          <span className="br-decision-card__model">
            <DecisionMark aria-hidden data-testid="decision-card-mark" />
            <span>{model}</span>
          </span>
        )}
        {tier && (
          <span className="br-decision-card__tier" data-tier={tier}>
            {tier === 'private' && <Lock aria-hidden />}
            {tier === 'private' ? DECISION_CARD_COPY.privateModel : DECISION_CARD_COPY.publicModel}
          </span>
        )}
        {latency && <span>{latency}</span>}
        <Button
          type="button"
          variant="link"
          size="xs"
          className="br-decision-card__history"
          aria-label={DECISION_CARD_COPY.historyLabel}
          onClick={() =>
            onOpenArtifact(
              // This one call, selected. Its source and turn too, which still
              // narrow the list on a daemon that predates `decisionId`.
              decisionHistoryArtifact(sessionId, {
                decisionId: record.id,
                origin: 'chat',
                ...(record.turnId ? { turnId: record.turnId } : {}),
              })
            )
          }
        >
          {DECISION_CARD_COPY.history}
        </Button>
      </div>
    </section>
  );
}

export interface DecisionCardProps {
  sessionId: string;
  decisionId: string;
  /** What the reply says in text, drawn while the row loads and if it cannot be read. */
  fallback: React.ReactNode;
  onOpenArtifact: (artifact: ArtifactSource) => void;
}

/**
 * The card for an assistant message whose `metadata.decisionId` is set: the reply
 * of a chat whose main model is a decision model. It reads that one row (cached
 * per id for the window) and falls back to the message's own text, which says the
 * same thing in words, while it loads or when it cannot.
 */
export function DecisionCard({
  sessionId,
  decisionId,
  fallback,
  onOpenArtifact,
}: DecisionCardProps) {
  const state = useDecisionRecord(sessionId, decisionId);
  if (state.status !== 'ready') {
    return (
      <div data-testid="decision-card-fallback" data-state={state.status}>
        {fallback}
      </div>
    );
  }
  return (
    <DecisionCardView record={state.record} sessionId={sessionId} onOpenArtifact={onOpenArtifact} />
  );
}
