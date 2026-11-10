import './decisionsChat.css';
import * as React from 'react';
import { AlertTriangle, Check, CircleIcon, Lock } from '../icons/app-icons';
import { ENTITY_ICONS } from '../icons/entity-icons';
import { Badge } from '../ui/badge';
import { Button } from '../ui/button';
import { Disclosure } from '../ui/disclosure';
import { InfoTip } from '../ui/info-tip';
import { Progress } from '../ui/progress';
import type { ArtifactSource } from '../artifacts/artifactTypes';
import {
  answerKey,
  answerProbability,
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
  type ProbabilitySource,
} from '../../utils/sessionDecisions';
import { orderedProbabilities } from './decisionWire';
import { useDecisionRecord } from './decisionChatApi';
import { decisionModelPlace } from './decisionWords';

const DecisionMark = ENTITY_ICONS.decision;

export const DECISION_CARD_COPY = {
  region: (question: string) => `Decision: ${question}`,
  probabilities: 'Probabilities',
  /**
   * Where an answer stands against its question's threshold. Neither word names a
   * direction: a `below` rule is met when P is under its number. And neither is a
   * gate's verdict: a decision chat's threshold marks an answer, so an answer that
   * does not meet it is an answer, not a failure (B-4). Phase gates keep their own
   * "Passed" / "Not passed".
   */
  meets: 'Meets the threshold',
  doesNotMeet: "Doesn't meet the threshold",
  threshold: (summary: string) => `Threshold: ${summary}`,
  got: (p: string) => `, got ${p}`,
  details: 'Details',
  uncalibrated: 'Uncalibrated',
  why: (reason: string) => `Why: ${reason}`,
  history: 'Decision history',
  historyLabel: 'Open this decision in the decision history',
  /** Visually hidden after the `Lock`: privacy is a shape, not a word. */
  privateModel: 'Private model',
  notDecided: 'Not decided',
  refused: 'Refused',
};

/**
 * What an uncalibrated probability means, said where it is (the "Uncalibrated"
 * badge's InfoTip), in at most one plain sentence.
 */
export function calibrationHelp(source: ProbabilitySource): string {
  if (source.kind === 'verbalized') {
    return 'The model wrote this number itself, so read it as a rough guide.';
  }
  if (source.kind === 'logprob') {
    return `Only ${formatProbability(source.mass)} of the probability fell on the options, so read it as a rough guide.`;
  }
  return 'This number has not been calibrated on your data, so read it as a rough guide.';
}

/**
 * One answered question up front: the question, the answer once, large, with that
 * answer's own P (R2), where it stands against a threshold (a word and a shape,
 * muted, never a cross) and, for an uncalibrated source, the "Uncalibrated" badge
 * whose InfoTip says what that means.
 */
function DecisionHeadline({
  decision,
  question,
}: {
  decision: Decision;
  question: NamedQuestion | undefined;
}) {
  const answerP = answerProbability(decision);
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
            data-meets={decision.passed ? 'true' : 'false'}
            data-testid="decision-verdict"
          >
            {decision.passed ? <Check aria-hidden /> : <CircleIcon aria-hidden />}
            {decision.passed ? DECISION_CARD_COPY.meets : DECISION_CARD_COPY.doesNotMeet}
          </span>
        )}
      </div>
      {!decision.calibrated && (
        <div className="br-decision-card__flags">
          <InfoTip
            label={DECISION_CARD_COPY.uncalibrated}
            help={calibrationHelp(decision.source)}
            asChild
          >
            <Badge data-testid="decision-calibration">{DECISION_CARD_COPY.uncalibrated}</Badge>
          </InfoTip>
        </div>
      )}
    </div>
  );
}

/** The bars, the threshold in its record form and a verbalized source's reason: Details. */
function DecisionFigures({
  decision,
  question,
}: {
  decision: Decision;
  question: NamedQuestion | undefined;
}) {
  const bars = orderedProbabilities(decision, question);
  const chosen = answerKey(decision.answer);
  const threshold = question?.threshold ?? null;
  const reason = decision.source.kind === 'verbalized' ? decision.source.reason?.trim() : null;
  return (
    <>
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
      {reason ? (
        <p className="br-decision-card__line" data-testid="decision-why">
          {DECISION_CARD_COPY.why(reason)}
        </p>
      ) : null}
    </>
  );
}

export interface DecisionCardViewProps {
  record: DecisionRecord;
  /** The chat the row belongs to, for the history link. */
  sessionId: string;
  onOpenArtifact: (artifact: ArtifactSource) => void;
}

/**
 * A decision chat's reply as a card, drawn from its decision-log row: each
 * question's headline up front, then one Details holding the bars, the
 * threshold, the model (with where it runs and privacy as the `Lock`), the
 * latency and the "Decision history" link. With several questions Details holds
 * one labelled group per question, in the headlines' order.
 */
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
  const place = ref ? decisionModelPlace(ref.provider, { tier }) : null;
  const latency = formatLatency(record.latencyMs ?? first?.latencyMs);
  const failed = record.outcome === 'error' || record.outcome === 'refused';
  const several = record.decisions.length > 1;
  return (
    <section
      className="br-decision-card br-fill-surface"
      aria-label={DECISION_CARD_COPY.region(regionQuestion)}
      data-testid="decision-card"
      data-outcome={record.outcome}
    >
      {record.decisions.map((decision) => (
        <DecisionHeadline
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
      <Disclosure label={DECISION_CARD_COPY.details} className="br-decision-card__details">
        <div className="br-decision-card__details-body" data-testid="decision-details">
          {record.decisions.map((decision) => {
            const question = questions.get(decision.question);
            if (!several) {
              return (
                <DecisionFigures key={decision.question} decision={decision} question={question} />
              );
            }
            const prompt = question?.instructions.trim() || decision.question;
            const answerP = answerProbability(decision);
            return (
              <section
                key={decision.question}
                className="br-decision-card__group"
                aria-label={prompt}
                data-testid="decision-group"
              >
                <p className="br-decision-card__question">{prompt}</p>
                <p className="br-decision-card__group-answer">
                  <span>{readableAnswer(decision, question)}</span>
                  {answerP !== null ? (
                    <span className="br-decision-card__answer-p">
                      P {formatProbability(answerP)}
                    </span>
                  ) : null}
                </p>
                <DecisionFigures decision={decision} question={question} />
              </section>
            );
          })}
          <div className="br-decision-card__meta" data-testid="decision-meta">
            {model && (
              <span className="br-decision-card__model">
                <DecisionMark aria-hidden data-testid="decision-card-mark" />
                <span>{place ? `${model} · ${place}` : model}</span>
                {tier === 'private' && (
                  <span className="br-decision-card__tier" data-tier={tier}>
                    <Lock aria-hidden />
                    <span className="sr-only">{DECISION_CARD_COPY.privateModel}</span>
                  </span>
                )}
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
        </div>
      </Disclosure>
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
