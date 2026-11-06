import './decisionsChat.css';
import { Button } from '../ui/button';
import { TranscriptRowSection } from '../TranscriptRow';
import type { ArtifactSource } from '../artifacts/artifactTypes';
import {
  decisionHistoryArtifact,
  decisionModelLabel,
  formatLatency,
  formatProbability,
} from '../../utils/sessionDecisions';
import {
  decideArguments,
  parseDecideBatchSummary,
  pluralize,
  type DecideQuestionSummary,
} from './decisionWire';

export const DECIDE_TOOL_COPY = {
  questions: 'Questions',
  result: 'Result',
  errors: 'Errors',
  history: 'Open this batch in the decision history',
  historyShort: 'Decision history',
  noSummary: 'The result carried no batch summary.',
  uncalibrated: 'uncalibrated',
  /** A question's type; a word the tool does not take is shown as it was written. */
  kind: (kind: string) => (kind === 'yes_no' ? 'yes or no' : kind),
  threshold: (p: number) => `threshold ${formatProbability(p)}`,
  /** Chunks the agent has to read before it reports them (not a clear no or pass). */
  review: (count: number) => `${count.toLocaleString('en-US')} to review`,
};

/** "Checking 600 chunks against 3 questions": the row's verb and what it counts. */
export function summarizeDecideCall(args: Record<string, unknown> | undefined): string {
  const { chunks, questions } = decideArguments(args);
  const subject = chunks !== null ? pluralize(chunks, 'chunk') : 'chunks';
  return questions.length > 0
    ? `Checking ${subject} against ${pluralize(questions.length, 'question')}`
    : `Checking ${subject} with a decision model`;
}

/** The row's meta once the batch answered: its errors, when there were any. */
export function decideCallMeta(toolResult: unknown): string | null {
  const summary = parseDecideBatchSummary(toolResult);
  return summary && summary.errors > 0 ? pluralize(summary.errors, 'error') : null;
}

const n = (value: number) => value.toLocaleString('en-US');

/**
 * One question's line: "yes 41, 6 to review, max p 0.98" for a yes or no
 * question, "rrms 300, spms 200, none 100, 12 passed, 9 to review, max p 0.91"
 * for a choice with a threshold. Counts only.
 */
function questionLine(question: DecideQuestionSummary): string {
  const parts: string[] = [];
  if (question.kind === 'yes_no') {
    if (question.yes !== null) parts.push(`yes ${n(question.yes)}`);
  } else {
    parts.push(...question.answerCounts.map(({ answer, count }) => `${answer} ${n(count)}`));
  }
  // With a threshold, how many passed; a yes count already says it when they agree.
  if (question.passed !== null && question.passed !== question.yes) {
    parts.push(`${n(question.passed)} passed`);
  }
  // Any type: a choice or score with a threshold has a review list too.
  if (question.review) parts.push(DECIDE_TOOL_COPY.review(question.review));
  if (question.maxP !== null) parts.push(`max p ${formatProbability(question.maxP)}`);
  return parts.join(', ');
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
 * The well of a `decisions__decide` tool row: the questions it asked, then once
 * it answered, per question how many chunks said yes, how many passed and the
 * highest probability, the distinct errors with how many chunks each hit, and a
 * link to the batch in the decision history (owner addendum A1). The chunks
 * themselves, which may number in the hundreds and hold clinical text, are never
 * listed here, and neither is the result's text for the model, which names chunk
 * ids.
 */
export function DecideToolSummary({
  sessionId,
  args,
  toolResult,
  onOpenArtifact,
}: DecideToolSummaryProps) {
  const asked = decideArguments(args);
  const summary = toolResult === undefined ? null : parseDecideBatchSummary(toolResult);
  const chunks = summary?.chunks ?? asked.chunks;
  const elapsed = formatLatency(summary?.elapsedMs);
  // Once the batch answered, each question's type as the daemon read it.
  const answeredKinds = new Map(
    summary?.questions.map((question) => [question.name, question.kind])
  );
  const head = [
    chunks !== null ? pluralize(chunks, 'chunk') : null,
    // Named as everywhere else: joey-1 for its Baseten slug, an Ollama tag as it is.
    summary?.model ? decisionModelLabel(summary.model) : null,
    summary && summary.errors > 0 ? pluralize(summary.errors, 'error') : null,
    summary?.calibrated === false ? DECIDE_TOOL_COPY.uncalibrated : null,
    elapsed,
  ].filter((part): part is string => part !== null);

  return (
    <div className="br-decide-summary" data-testid="decide-tool-summary">
      {asked.questions.length > 0 && (
        <TranscriptRowSection label={DECIDE_TOOL_COPY.questions}>
          <ul className="br-decide-summary__list">
            {asked.questions.map((question) => (
              <li key={question.name} className="br-decide-summary__question">
                <span className="br-decide-summary__name">{question.name}</span>
                <span className="br-decide-summary__detail">
                  {[
                    question.instructions,
                    DECIDE_TOOL_COPY.kind(answeredKinds.get(question.name) ?? question.kind),
                    question.threshold !== null
                      ? DECIDE_TOOL_COPY.threshold(question.threshold)
                      : null,
                  ]
                    .filter(Boolean)
                    .join(' · ')}
                </span>
              </li>
            ))}
          </ul>
        </TranscriptRowSection>
      )}
      {toolResult !== undefined && (
        <TranscriptRowSection label={DECIDE_TOOL_COPY.result}>
          {head.length > 0 && (
            <p className="br-decide-summary__head" data-testid="decide-tool-head">
              {head.join(' · ')}
            </p>
          )}
          {summary && summary.questions.length > 0 && (
            <ul className="br-decide-summary__list" data-testid="decide-tool-questions">
              {summary.questions.map((question) => (
                <li key={question.name} className="br-decide-summary__question">
                  <span className="br-decide-summary__name">{question.name}</span>
                  <span className="br-decide-summary__detail">{questionLine(question)}</span>
                </li>
              ))}
            </ul>
          )}
          {!summary && <p className="br-decide-summary__head">{DECIDE_TOOL_COPY.noSummary}</p>}
        </TranscriptRowSection>
      )}
      {summary && summary.errorGroups.length > 0 && (
        <TranscriptRowSection label={DECIDE_TOOL_COPY.errors}>
          <ul className="br-decide-summary__list" data-testid="decide-tool-errors">
            {summary.errorGroups.map((group) => (
              <li key={group.message} className="br-decide-summary__question">
                <span className="br-decide-summary__name">{pluralize(group.chunks, 'chunk')}</span>
                <span className="br-decide-summary__detail">{group.message}</span>
              </li>
            ))}
          </ul>
        </TranscriptRowSection>
      )}
      {toolResult !== undefined && sessionId && (
        <Button
          type="button"
          variant="link"
          size="xs"
          className="br-decide-summary__history"
          aria-label={DECIDE_TOOL_COPY.history}
          onClick={() =>
            onOpenArtifact(
              decisionHistoryArtifact(
                sessionId,
                summary ? { origin: 'tool', batchId: summary.batchId } : { origin: 'tool' }
              )
            )
          }
        >
          {DECIDE_TOOL_COPY.historyShort}
        </Button>
      )}
    </div>
  );
}
