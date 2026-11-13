import './decisionsChat.css';
import './decisionsPanel.css';
import { useRef } from 'react';
import { AlertCircle, Lock } from '../icons/app-icons';
import { InfoTip } from '../ui/info-tip';
import { PRIVATE_MARK_INK } from '../ui/PrivacyBadge';
import { TranscriptRowSection } from '../TranscriptRow';
import type { ArtifactSource } from '../artifacts/artifactTypes';
import { decisionHistoryArtifact, formatProbability } from '../../utils/sessionDecisions';
import { DecisionHistoryLink } from './DecisionHistoryLink';
import {
  decideArguments,
  parseDecideBatchSummary,
  pluralize,
  type DecideBatchSummary,
  type DecideEscalatedAnswer,
  type DecideEscalation,
  type DecideQuestionSummary,
} from './decisionWire';
import { CALIBRATION_COPY, decisionModelDisplay } from './decisionWords';
import { useRevealWhenOpened } from './useRevealWhenOpened';

/** The card names at most this many questions; the rest are in the history. */
export const DECIDE_CARD_QUESTIONS_SHOWN = 3;

/** The open well lists at most this many escalated chunks; the rest are in the history. */
export const DECIDE_CARD_ESCALATED_SHOWN = 10;

export const DECIDE_TOOL_COPY = {
  errors: 'Errors',
  history: 'Open this decide call in the decision history',
  noSummary: 'The result carried no batch summary.',
  question: (n: number) => `Question ${n}`,
  /** Fewer chunks checked than sent, when the errors do not account for it. */
  checked: (decided: number, chunks: number) =>
    `${decided.toLocaleString('en-US')} of ${pluralize(chunks, 'chunk')} checked`,
  /** Chunks the agent has to read before it reports them (not a clear no or pass). */
  review: (count: number) => `${count.toLocaleString('en-US')} to review`,
  /**
   * The review count beside answer counts it is PART of (R1-5): "4 of them to review",
   * so `1 yes · 19 no · 4 of them to review` cannot be read as 24 chunks.
   */
  reviewOfThem: (count: number) => `${count.toLocaleString('en-US')} of them to review`,
  /** One answer's count, the number first: `19 no`, `300 rrms`. */
  answerCount: (answer: string, count: number) => `${count.toLocaleString('en-US')} ${answer}`,
  /** A vote's counts are its majority's (G-1). */
  majority: 'Majority',
  tied: (count: number) => `${count.toLocaleString('en-US')} tied`,
  agreed: (agreed: number, compared: number) =>
    `the models agreed on ${agreed.toLocaleString('en-US')} of ${compared.toLocaleString('en-US')}`,
  /** The well's first line for a vote: how many models, then their names. */
  vote: (labels: readonly string[]) => `Vote of ${labels.length} models: ${labels.join(', ')}`,
  modelsSilent: (n: number) =>
    `${n.toLocaleString('en-US')} ${n === 1 ? 'model' : 'models'} answered nothing`,
  more: (n: number) =>
    `${n.toLocaleString('en-US')} more ${n === 1 ? 'question' : 'questions'} in the decision history`,
  calibration: 'Probabilities',
  privateModel: 'Private model',
  /** `escalate`'s one line: `8 of 30 escalated to clef-flash:latest`. */
  escalated: (escalated: number, of: number, label: string) =>
    `${escalated.toLocaleString('en-US')} of ${of.toLocaleString('en-US')} escalated to ${label}`,
  /** Escalated chunks the second model failed to answer; the first answer stands. */
  escalationFailed: (count: number) => `${count.toLocaleString('en-US')} failed`,
  /**
   * Escalated chunks too long for the second model's window, never sent to it: not a
   * failure, so said apart from one (`2 too long for elephant-1`).
   */
  escalationTooLong: (count: number, label: string) =>
    `${count.toLocaleString('en-US')} too long for ${label}`,
  escalation: 'Escalation',
  /**
   * The escalation line's InfoTip: the bound and that the second model's answers are
   * final, then what it changed, or, when it failed some, that the first answers stand.
   */
  escalationTip: (escalation: {
    label: string;
    firstLabel: string;
    below: number | null;
    answers: number;
    changed: number;
    errors: number;
  }) => {
    const below = formatProbability(escalation.below);
    const asked = below
      ? `Answers with a top probability under ${below} were asked again of ${escalation.label}, and its answers are final.`
      : `The unsure answers were asked again of ${escalation.label}, and its answers are final.`;
    const after =
      escalation.errors > 0
        ? `Where it gave no answer, ${escalation.firstLabel}'s answer stands.`
        : escalation.answers > 0
          ? `It changed ${escalation.changed.toLocaleString('en-US')} of ${escalation.answers.toLocaleString('en-US')}.`
          : 'No answer was that unsure, so it was asked nothing.';
    return `${asked} ${after}`;
  },
  /** A chunk by its place in the call, as the decision history numbers it. */
  chunk: (n: number) => `Chunk ${n.toLocaleString('en-US')}`,
  /** The closed row's meta: `10 yes, 8 escalated`. */
  escalatedMeta: (count: number) => `${count.toLocaleString('en-US')} escalated`,
  escalatedSection: 'Escalated',
  /** One escalated answer: the final answer, then the first model's. */
  escalatedAnswer: (
    label: string,
    p: string | null,
    firstModel: string,
    firstLabel: string,
    firstP: string | null
  ) =>
    `${label}${p ? ` ${p}` : ''}, was ${firstLabel}${firstP ? ` ${firstP}` : ''} by ${firstModel}`,
  /** An answer the second model did not give, because it failed. */
  escalatedKept: (label: string, firstModel: string) =>
    `${label} by ${firstModel}: the second model gave no answer`,
  /** An answer the second model was never asked for: the chunk was too long for it. */
  escalatedTooLong: (label: string, firstModel: string, secondModel: string) =>
    `${label} by ${firstModel}: too long for ${secondModel}`,
  escalatedMore: (n: number) =>
    `${n.toLocaleString('en-US')} more ${n === 1 ? 'chunk' : 'chunks'} in the decision history`,
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
 * for ONE yes or no question its yes count, then what is left to review, the chunks
 * `escalate` asked again and the errors (`10 yes`, `41 yes, 6 to review, 2 errors`,
 * `10 yes, 8 escalated`); for a choice, a score or several questions only the
 * escalated count and the errors. `null` when there is nothing to say.
 */
export function decideCallMeta(toolResult: unknown): string | null {
  const summary = parseDecideBatchSummary(toolResult);
  if (!summary) return null;
  const parts: string[] = [];
  const only = summary.questions.length === 1 ? summary.questions[0] : null;
  if (only?.vote) {
    // A vote: its majority's yes count (G-1).
    const yes = only.vote.majority.find(({ answer }) => answer === 'yes');
    if (yes) parts.push(`${n(yes.count)} yes by majority`);
  } else if (only?.kind === 'yes_no' && only.groups) {
    const yes = only.groups.yesConfident + only.groups.yesReview;
    const review = only.groups.yesReview + only.groups.noReview;
    parts.push(`${n(yes)} yes`);
    if (review) parts.push(DECIDE_TOOL_COPY.review(review));
  } else if (only && only.kind === 'yes_no' && only.yes !== null) {
    parts.push(`${n(only.yes)} yes`);
    if (only.review) parts.push(DECIDE_TOOL_COPY.review(only.review));
  }
  if (summary.escalation?.escalated) {
    parts.push(DECIDE_TOOL_COPY.escalatedMeta(summary.escalation.escalated));
  }
  if (summary.errors > 0) parts.push(pluralize(summary.errors, 'error'));
  return parts.length > 0 ? parts.join(', ') : null;
}

/**
 * One question's counts line, answers that add up to the chunks decided and then the
 * review count, said as part of them (R1-5): `1 yes · 19 no · 4 of them to review`
 * for a yes or no question (from the daemon's four groups when it sends them, where a
 * yes is a pass); `300 rrms · 200 spms · 98 none · 12 of them to review` for a choice
 * or a score, in option order, with or without a threshold (its groups give only the
 * review count). A vote of several models (G-1) shows its majority and
 * how often the models agreed: `Majority 7 yes · 13 no · the models agreed on 19 of 20`.
 * The type word, the threshold and the passed count are not on the card: the history's
 * rows and detail say them.
 */
export function decideCountsLine(question: DecideQuestionSummary): string {
  const answers = (list: ReadonlyArray<{ answer: string; count: number }>) =>
    list.map(({ answer, count }) => DECIDE_TOOL_COPY.answerCount(answer, count));
  if (question.vote) {
    const { majority, ties, agreed, compared } = question.vote;
    const parts = answers(majority);
    if (parts.length > 0) parts[0] = `${DECIDE_TOOL_COPY.majority} ${parts[0]}`;
    if (ties) parts.push(DECIDE_TOOL_COPY.tied(ties));
    if (compared) parts.push(DECIDE_TOOL_COPY.agreed(agreed, compared));
    return parts.join(' · ');
  }
  const parts: string[] = [];
  let review = question.groups
    ? question.groups.yesReview + question.groups.noReview
    : question.review;
  if (question.kind === 'yes_no' && question.groups) {
    // A yes or no question's four groups, where a yes is a pass. A choice or a
    // score with a threshold has the groups too, but its answers are its options:
    // a three-way choice drawn as "10 yes · 20 no" named answers it never offered.
    const { yesConfident, yesReview, noReview, noConfident } = question.groups;
    parts.push(...answers([{ answer: 'yes', count: yesConfident + yesReview }]));
    parts.push(...answers([{ answer: 'no', count: noReview + noConfident }]));
  } else if (question.kind === 'yes_no' && question.yes !== null) {
    parts.push(DECIDE_TOOL_COPY.answerCount('yes', question.yes));
    if (question.decided !== null && question.decided >= question.yes) {
      parts.push(DECIDE_TOOL_COPY.answerCount('no', question.decided - question.yes));
    }
  } else {
    parts.push(...answers(question.answerCounts));
  }
  // The review count is a part of the answers above, never a group beside them.
  if (review) {
    parts.push(
      parts.length > 0 ? DECIDE_TOOL_COPY.reviewOfThem(review) : DECIDE_TOOL_COPY.review(review)
    );
  }
  return parts.join(' · ');
}

const joinParts = (parts: Array<string | null>) =>
  parts.filter((part): part is string => !!part).join(' · ');

/**
 * The well's first, muted line: the model and where it runs, then the errors. The
 * model half is apart so a private model's padlock sits right after its name (R1-10:
 * the one privacy mark, in `PRIVATE_MARK_INK`, as on the decision card and the chip).
 */
function headLine(summary: DecideBatchSummary): {
  model: string | null;
  isPrivate: boolean;
  rest: string;
} {
  if (summary.voteModels) {
    const silent = summary.voteModels.filter((model) => model.error !== null).length;
    return {
      model: null,
      isPrivate: false,
      rest: joinParts([
        DECIDE_TOOL_COPY.vote(summary.voteModels.map((model) => model.label)),
        silent > 0 ? DECIDE_TOOL_COPY.modelsSilent(silent) : null,
      ]),
    };
  }
  const unexplained =
    summary.chunks !== null &&
    summary.decided !== null &&
    summary.decided + summary.errors < summary.chunks;
  const model = decisionModelDisplay(summary.model, summary.tier);
  return {
    model,
    isPrivate: !!model && summary.tier === 'private',
    rest: joinParts([
      unexplained ? DECIDE_TOOL_COPY.checked(summary.decided!, summary.chunks!) : null,
      summary.errors > 0 ? pluralize(summary.errors, 'error') : null,
    ]),
  };
}

/** One escalated answer in words, its question named when the call asked several. */
function escalatedAnswerText(
  answer: DecideEscalatedAnswer,
  escalation: DecideEscalation,
  question: string | null
): string {
  const prefix = question ? `${question}: ` : '';
  if (answer.error && answer.tooLarge) {
    return (
      prefix +
      DECIDE_TOOL_COPY.escalatedTooLong(answer.label, escalation.firstLabel, escalation.label)
    );
  }
  if (answer.error) {
    return prefix + DECIDE_TOOL_COPY.escalatedKept(answer.label, escalation.firstLabel);
  }
  return (
    prefix +
    DECIDE_TOOL_COPY.escalatedAnswer(
      answer.label,
      formatProbability(answer.p) || null,
      escalation.firstLabel,
      answer.firstLabel,
      formatProbability(answer.firstP) || null
    )
  );
}

/**
 * The open well's escalation line: `8 of 30 escalated to clef-flash:latest`, a private
 * model's padlock after its name, then the chunks it never answered, by why: those too
 * long for its window (`2 too long for clef-flash:latest`, never sent to it) apart from
 * those it failed (`1 failed`). Then an InfoTip (hover peeks, click pins) with the
 * bound and what the second model changed. The well is the row's own disclosure, so
 * this never shows while the row is closed; the closed row's meta says only
 * `8 escalated`.
 */
function EscalationLine({ escalation }: { escalation: DecideEscalation }) {
  return (
    <p
      className="br-decide-card__line br-decide-card__estimate"
      data-testid="decide-tool-escalation"
    >
      <span data-testid="decide-tool-escalation-text">
        {DECIDE_TOOL_COPY.escalated(
          escalation.escalated,
          escalation.firstDecided,
          escalation.label
        )}
        {escalation.tier === 'private' ? (
          <Lock
            role="img"
            aria-label={DECIDE_TOOL_COPY.privateModel}
            className={`ml-1 inline-block size-3 align-[-1px] ${PRIVATE_MARK_INK}`}
          />
        ) : null}
        {escalation.tooLong > 0
          ? ` · ${DECIDE_TOOL_COPY.escalationTooLong(escalation.tooLong, escalation.label)}`
          : null}
        {escalation.failed > 0
          ? ` · ${DECIDE_TOOL_COPY.escalationFailed(escalation.failed)}`
          : null}
      </span>
      <InfoTip
        label={DECIDE_TOOL_COPY.escalation}
        help={DECIDE_TOOL_COPY.escalationTip(escalation)}
      />
    </p>
  );
}

/**
 * The chunks `escalate` asked again, each named by its place in the call ("Chunk 4",
 * the decision history's own number for it, never the id the agent gave it) with its
 * final answer beside the first model's, at most `DECIDE_CARD_ESCALATED_SHOWN` of them,
 * then how many more the decision history holds. Never a row per chunk of the batch:
 * only those asked again.
 */
function EscalatedChunks({
  escalation,
  chunkName,
  questionText,
}: {
  escalation: DecideEscalation;
  chunkName: (chunkId: string) => string;
  questionText: (name: string) => string;
}) {
  const shown = escalation.items.slice(0, DECIDE_CARD_ESCALATED_SHOWN);
  if (shown.length === 0) return null;
  const more = escalation.items.length - shown.length;
  const several =
    new Set(escalation.items.flatMap((item) => item.answers.map((answer) => answer.question)))
      .size > 1;
  return (
    <TranscriptRowSection label={DECIDE_TOOL_COPY.escalatedSection}>
      <ul className="br-decide-card__errors" data-testid="decide-tool-escalated">
        {shown.map((item) => (
          <li key={item.chunkId} className="br-decide-card__error">
            <span className="br-decide-card__error-count">{chunkName(item.chunkId)}</span>
            <span className="br-decide-card__error-message">
              {item.answers
                .map((answer) =>
                  escalatedAnswerText(
                    answer,
                    escalation,
                    several ? questionText(answer.question) : null
                  )
                )
                .join('; ')}
            </span>
          </li>
        ))}
      </ul>
      {more > 0 ? (
        <p className="br-decide-card__more" data-testid="decide-tool-escalated-more">
          {DECIDE_TOOL_COPY.escalatedMore(more)}
        </p>
      ) : null}
    </TranscriptRowSection>
  );
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
 * The well of a `decisions__decide` tool row (F-7): the model, the escalation's one
 * line when the call escalated, then per question (at most three) the question in
 * plain words and its counts, the chunks `escalate` asked again (at most ten, by their
 * number in the call, with the final answer beside the first model's), the distinct errors with how many
 * chunks each hit, and the link to this call in the decision history (owner addendum
 * A1). Question names, the type, the threshold, the passed count and the time are not
 * here: the history shows them. The chunks' text, which may hold clinical notes, is
 * never shown, and neither is the result's text for the model.
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
  const head = summary ? headLine(summary) : null;
  // Mounted only while the row is open, so this is the person opening it (G-7): a row at
  // the end of the chat would otherwise open under the composer, link and all.
  const ref = useRef<HTMLDivElement | null>(null);
  useRevealWhenOpened(ref);

  return (
    <div ref={ref} className="br-decide-card" data-testid="decide-tool-summary">
      {head && (head.model || head.rest) ? (
        <p className="br-decide-card__line" data-testid="decide-tool-head">
          {head.model}
          {head.isPrivate ? (
            <Lock
              role="img"
              aria-label={DECIDE_TOOL_COPY.privateModel}
              className={`ml-1 inline-block size-3 align-[-1px] ${PRIVATE_MARK_INK}`}
              data-testid="decide-tool-private"
            />
          ) : null}
          {head.model && head.rest ? ' · ' : null}
          {head.rest}
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
      {summary?.escalation ? <EscalationLine escalation={summary.escalation} /> : null}
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
      {summary?.escalation ? (
        <EscalatedChunks
          escalation={summary.escalation}
          chunkName={(chunkId) => {
            const place = asked.chunkIds.indexOf(chunkId);
            // The call's own arguments always hold it; the id is only a fallback.
            return place >= 0 ? DECIDE_TOOL_COPY.chunk(place + 1) : chunkId;
          }}
          questionText={(name) => {
            const index = questions.findIndex((question) => question.name === name);
            return textOf.get(name)?.trim() || DECIDE_TOOL_COPY.question(index + 1);
          }}
        />
      ) : null}
      {summary && summary.errorGroups.length > 0 && (
        <TranscriptRowSection label={DECIDE_TOOL_COPY.errors}>
          <ul className="br-decide-card__errors" data-testid="decide-tool-errors">
            {summary.errorGroups.map((group) => (
              <li key={group.message} className="br-decide-card__error">
                {/* Red is rare (design.md 9.14): a chunk the model failed is the
                    failure glyph and words in ink, never a status hue. */}
                <AlertCircle aria-hidden data-testid="decide-tool-error-glyph" />
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
