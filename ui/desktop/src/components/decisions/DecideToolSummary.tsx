import './decisionsChat.css';
import './decisionsPanel.css';
import { useRef } from 'react';
import { InfoTip } from '../ui/info-tip';
import { TranscriptRowSection } from '../TranscriptRow';
import type { ArtifactSource } from '../artifacts/artifactTypes';
import { decisionHistoryArtifact } from '../../utils/sessionDecisions';
import { DecisionHistoryLink } from './DecisionHistoryLink';
import {
  decideArguments,
  parseDecideBatchSummary,
  pluralize,
  type DecideBatchSummary,
  type DecideQuestionSummary,
} from './decisionWire';
import { CALIBRATION_COPY, decisionModelDisplay } from './decisionWords';
import { useRevealWhenOpened } from './useRevealWhenOpened';

/** The card names at most this many questions; the rest are in the history. */
export const DECIDE_CARD_QUESTIONS_SHOWN = 3;

export const DECIDE_TOOL_COPY = {
  errors: 'Errors',
  history: 'Open this decide call in the decision history',
  noSummary: 'The result carried no batch summary.',
  question: (n: number) => `Question ${n}`,
  /** Fewer chunks checked than sent, when the errors do not account for it. */
  checked: (decided: number, chunks: number) =>
    `${decided.toLocaleString('en-US')} of ${pluralize(chunks, 'chunk')} checked`,
  /** `10 of 30 yes`: the denominator is the chunks that answered the question. */
  yesOf: (yes: number, of: number) =>
    `${yes.toLocaleString('en-US')} of ${of.toLocaleString('en-US')} yes`,
  /** Chunks the agent has to read before it reports them (not a clear no or pass). */
  review: (count: number) => `${count.toLocaleString('en-US')} to review`,
  more: (n: number) =>
    `${n.toLocaleString('en-US')} more ${n === 1 ? 'question' : 'questions'} in the decision history`,
  calibration: 'Probabilities',
};

const n = (value: number) => value.toLocaleString('en-US');

/** "Checking 600 chunks against 3 questions": the row's verb and what it counts. */
export function summarizeDecideCall(args: Record<string, unknown> | undefined): string {
  const { chunks, questions } = decideArguments(args);
  const subject = chunks !== null ? pluralize(chunks, 'chunk') : 'chunks';
  return questions.length > 0
    ? `Checking ${subject} against ${pluralize(questions.length, 'question')}`
    : `Checking ${subject} with a decision model`;
}

/**
 * The row's meta once the batch answered, which never truncates, so it stays short:
 * for ONE yes or no question its yes count, then what is left to review and the
 * errors (`10 yes`, `41 yes, 6 to review, 2 errors`); for a choice, a score or
 * several questions only the errors. `null` when there is nothing to say.
 */
export function decideCallMeta(toolResult: unknown): string | null {
  const summary = parseDecideBatchSummary(toolResult);
  if (!summary) return null;
  const parts: string[] = [];
  const only = summary.questions.length === 1 ? summary.questions[0] : null;
  if (only && only.kind === 'yes_no' && only.yes !== null) {
    parts.push(`${n(only.yes)} yes`);
    if (only.review) parts.push(DECIDE_TOOL_COPY.review(only.review));
  }
  if (summary.errors > 0) parts.push(pluralize(summary.errors, 'error'));
  return parts.length > 0 ? parts.join(', ') : null;
}

/**
 * One question's counts line: `10 of 30 yes · 6 to review` for a yes or no question;
 * `rrms 300 · spms 200 · none 98 · 12 to review` for a choice or a score, in option
 * order. The type word, the threshold and the passed count are not on the card: the
 * history's rows and detail say them.
 */
export function decideCountsLine(question: DecideQuestionSummary): string {
  const parts: string[] = [];
  if (question.kind === 'yes_no') {
    if (question.yes !== null) {
      parts.push(
        question.decided !== null
          ? DECIDE_TOOL_COPY.yesOf(question.yes, question.decided)
          : `${n(question.yes)} yes`
      );
    }
  } else {
    parts.push(...question.answerCounts.map(({ answer, count }) => `${answer} ${n(count)}`));
  }
  if (question.review) parts.push(DECIDE_TOOL_COPY.review(question.review));
  return parts.join(' · ');
}

/** The well's first, muted line: the model and where it runs, then the errors. */
function headLine(summary: DecideBatchSummary): string {
  const unexplained =
    summary.chunks !== null &&
    summary.decided !== null &&
    summary.decided + summary.errors < summary.chunks;
  return [
    decisionModelDisplay(summary.model, summary.tier),
    unexplained ? DECIDE_TOOL_COPY.checked(summary.decided!, summary.chunks!) : null,
    summary.errors > 0 ? pluralize(summary.errors, 'error') : null,
  ]
    .filter((part): part is string => !!part)
    .join(' · ');
}

export interface DecideToolSummaryProps {
  /** The chat, for the history link. A surface with no chat draws no link. */
  sessionId?: string;
  args: Record<string, unknown>;
  /** The tool result, once the batch answered. */
  toolResult?: unknown;
  onOpenArtifact: (artifact: ArtifactSource) => void;
}

/**
 * The well of a `decisions__decide` tool row (F-7): the model, then per question (at
 * most three) the question in plain words and its counts, the distinct errors with
 * how many chunks each hit, and the link to this call in the decision history (owner
 * addendum A1). Question names, the type, the threshold, the passed count and the
 * time are not here: the history shows them. The chunks themselves, which may number
 * in the hundreds and hold clinical text, are never listed, and neither is the
 * result's text for the model, which names chunk ids.
 */
export function DecideToolSummary({
  sessionId,
  args,
  toolResult,
  onOpenArtifact,
}: DecideToolSummaryProps) {
  const asked = decideArguments(args);
  const summary = toolResult === undefined ? null : parseDecideBatchSummary(toolResult);
  const textOf = new Map(asked.questions.map((question) => [question.name, question.instructions]));
  // Once the batch answered, its questions as the daemon ran them; before, the call's own.
  const questions: Array<{ name: string; counts: string | null }> = summary
    ? summary.questions.map((question) => ({
        name: question.name,
        counts: decideCountsLine(question) || null,
      }))
    : asked.questions.map((question) => ({ name: question.name, counts: null }));
  const shown = questions.slice(0, DECIDE_CARD_QUESTIONS_SHOWN);
  const more = questions.length - shown.length;
  const head = summary ? headLine(summary) : '';
  // Mounted only while the row is open, so this is the person opening it (G-7): a row at
  // the end of the chat would otherwise open under the composer, link and all.
  const ref = useRef<HTMLDivElement | null>(null);
  useRevealWhenOpened(ref);

  return (
    <div ref={ref} className="br-decide-card" data-testid="decide-tool-summary">
      {head ? (
        <p className="br-decide-card__line" data-testid="decide-tool-head">
          {head}
        </p>
      ) : null}
      {summary?.calibrated === false ? (
        <p
          className="br-decide-card__line br-decide-card__estimate"
          data-testid="decide-tool-estimate"
        >
          {CALIBRATION_COPY.estimate}
          <InfoTip label={DECIDE_TOOL_COPY.calibration} help={CALIBRATION_COPY.estimateTip} />
        </p>
      ) : null}
      {shown.length > 0 ? (
        <ul className="br-decide-card__questions" data-testid="decide-tool-questions">
          {shown.map((question, index) => (
            <li key={question.name} className="br-decide-card__question">
              <span className="br-decide-card__text">
                {textOf.get(question.name)?.trim() || DECIDE_TOOL_COPY.question(index + 1)}
              </span>
              {question.counts ? (
                <span className="br-decide-card__counts">{question.counts}</span>
              ) : null}
            </li>
          ))}
        </ul>
      ) : null}
      {more > 0 ? (
        <p className="br-decide-card__more" data-testid="decide-tool-more">
          {DECIDE_TOOL_COPY.more(more)}
        </p>
      ) : null}
      {toolResult !== undefined && !summary ? (
        <p className="br-decide-card__line">{DECIDE_TOOL_COPY.noSummary}</p>
      ) : null}
      {summary && summary.errorGroups.length > 0 && (
        <TranscriptRowSection label={DECIDE_TOOL_COPY.errors}>
          <ul className="br-decide-card__errors" data-testid="decide-tool-errors">
            {summary.errorGroups.map((group) => (
              <li key={group.message} className="br-decide-card__error">
                <span className="br-decide-card__error-count">
                  {pluralize(group.chunks, 'chunk')}
                </span>
                <span className="br-decide-card__error-message">{group.message}</span>
              </li>
            ))}
          </ul>
        </TranscriptRowSection>
      )}
      {toolResult !== undefined && sessionId ? (
        <DecisionHistoryLink
          label={DECIDE_TOOL_COPY.history}
          onOpen={() =>
            onOpenArtifact(
              decisionHistoryArtifact(
                sessionId,
                summary ? { origin: 'tool', batchId: summary.batchId } : { origin: 'tool' }
              )
            )
          }
        />
      ) : null}
    </div>
  );
}
