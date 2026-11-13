import './decisionsChat.css';
import * as React from 'react';
import { AlertCircle, Check, Plus, X } from '../icons/app-icons';
import { Button } from '../ui/button';
import { Disclosure } from '../ui/disclosure';
import { InfoTip } from '../ui/info-tip';
import { Input } from '../ui/input';
import { SegmentedControl } from '../ui/segmented-control';
import { Select } from '../ui/Select';
import { questionToWire, type Question, type Threshold } from '../../utils/sessionDecisions';
import { fetchDecisionChatQuestion, saveDecisionChatQuestion } from './decisionChatApi';
import { ruleInWords } from './decisionWords';

export const DECISION_COMPOSER_COPY = {
  region: 'Decision question',
  question: 'Question',
  /** By answer kind: an example that cannot be mistaken for a kept value (B-5). */
  questionPlaceholder: {
    binary: 'For example: Does this note mention a fever?',
    choice: 'For example: Which MS course does this note describe?',
    score: 'For example: How severe is the pain this note describes?',
  },
  options: 'Question options',
  kind: 'Answer',
  kinds: { binary: 'Yes or no', choice: 'Choice', score: 'Score' },
  /** The closed Question options' summary: what kind of answer, in words. */
  kindSummary: {
    binary: () => 'Yes or no',
    choice: (n: number) => `Choice of ${n}`,
    score: (n: number) => `Score, ${n} ${n === 1 ? 'level' : 'levels'}`,
  },
  optionsLabel: 'Options',
  option: (n: number) => `Option ${n}`,
  removeOption: (name: string) => `Remove option ${name}`,
  addOption: 'Add option',
  levels: 'Levels, lowest first',
  level: (n: number) => `Level ${n}`,
  removeLevel: (name: string) => `Remove level ${name}`,
  addLevel: 'Add level',
  threshold: 'Threshold',
  thresholdInfo: 'Optional. With a threshold, each answer shows whether it meets it.',
  thresholdPlaceholder: 'None',
  thresholdOnWord: 'on',
  thresholdOn: 'Compared with',
  atLeast: (level: string) => `At least ${level}`,
  /** The live sentence under a threshold: the card's own words, never a gate's. */
  meetsSentence: (phrase: string) => `Meets the threshold ${phrase}.`,
  placeholder: 'Paste the text to decide on',
  draftNote:
    'Start your message with a question line, such as “Q: Is the form signed?”, then paste the text.',
  saving: 'Saving…',
  saved: 'Saved',
  loading: 'Reading this chat’s question…',
  needsQuestion: 'Write the question to ask of each message.',
  needsOptions: 'Name at least 2 different options.',
  needsLevels: 'Give every level a label.',
  badThreshold: 'The threshold is a number from 0 to 1.',
  keptRule: (phrase: string) =>
    `Meets the threshold ${phrase}, as set outside this chat. Clear the threshold to set a new one.`,
  refused: (message: string | null) =>
    message ?? 'Could not save the question. A “Q:” line at the start of your message still works.',
} as const;

/** The placeholder the composer's text box shows in a decision chat. */
export const DECISION_CHAT_PLACEHOLDER = DECISION_COMPOSER_COPY.placeholder;

type Kind = 'binary' | 'choice' | 'score';

interface Entry {
  id: number;
  text: string;
}

export interface DecisionDraft {
  instructions: string;
  kind: Kind;
  options: Entry[];
  levels: Entry[];
  /** The threshold as typed: '' is none. */
  threshold: string;
  /** The option name, or the level index, a choice or score threshold is compared with. */
  target: string;
  /**
   * What a stored threshold says that these fields do not edit, kept so a save leaves
   * the rule as it was: it passes below `threshold` (`below`), and a yes/no question's
   * threshold is on P(no) (`binarySide`). Both are set over HTTP or the CLI, shown by
   * a line under the fields, and cleared with the threshold.
   */
  below: boolean;
  binarySide: 'yes' | 'no';
}

let nextEntryId = 1;
const entry = (value = ''): Entry => ({ id: nextEntryId++, text: value });

export function emptyDraft(): DecisionDraft {
  return {
    instructions: '',
    kind: 'binary',
    options: [entry(), entry()],
    levels: [entry(), entry(), entry()],
    threshold: '',
    target: '',
    below: false,
    binarySide: 'yes',
  };
}

/** A stored question, as the fields edit it. */
export function draftFromQuestion(question: Question | null): DecisionDraft {
  const draft = emptyDraft();
  if (!question) return draft;
  draft.instructions = question.instructions;
  if (question.kind.type === 'choice') {
    draft.kind = 'choice';
    draft.options = question.kind.options.map((option) => entry(option.name));
  } else if (question.kind.type === 'score') {
    draft.kind = 'score';
    draft.levels = question.kind.levels.map((label) => entry(label));
  }
  const threshold = question.threshold;
  if (threshold) draft.threshold = String(threshold.p);
  if (threshold?.type === 'option') draft.target = threshold.name;
  if (threshold?.type === 'at_least_level') draft.target = String(threshold.level);
  if (threshold?.type === 'no') draft.binarySide = 'no';
  draft.below = threshold?.below === true;
  return draft;
}

export type DraftVerdict = { ok: true; question: Question } | { ok: false; problem: string };

/** The fields as the question the route stores, or the one thing still missing. */
export function questionFromDraft(draft: DecisionDraft): DraftVerdict {
  const instructions = draft.instructions.trim();
  if (!instructions) return { ok: false, problem: DECISION_COMPOSER_COPY.needsQuestion };
  const p = draft.threshold.trim() === '' ? null : Number(draft.threshold);
  if (p !== null && (!Number.isFinite(p) || p < 0 || p > 1)) {
    return { ok: false, problem: DECISION_COMPOSER_COPY.badThreshold };
  }
  const kept = (threshold: Threshold): Threshold =>
    draft.below ? { ...threshold, below: true } : threshold;
  if (draft.kind === 'binary') {
    return {
      ok: true,
      question: {
        instructions,
        kind: { type: 'binary' },
        threshold: p !== null ? kept({ type: draft.binarySide, p }) : null,
      },
    };
  }
  if (draft.kind === 'choice') {
    const names = draft.options.map((option) => option.text.trim());
    const folded = new Set(names.map((name) => name.toLowerCase()));
    if (
      names.length < 2 ||
      names.length > 26 ||
      names.some((name) => !name) ||
      folded.size !== names.length
    ) {
      return { ok: false, problem: DECISION_COMPOSER_COPY.needsOptions };
    }
    const target = names.includes(draft.target) ? draft.target : names[0];
    return {
      ok: true,
      question: {
        instructions,
        kind: { type: 'choice', options: names.map((name) => ({ name, description: null })) },
        threshold: p !== null ? kept({ type: 'option', name: target, p }) : null,
      },
    };
  }
  const levels = draft.levels.map((level) => level.text.trim());
  if (levels.length < 1 || levels.length > 10 || levels.some((level) => !level)) {
    return { ok: false, problem: DECISION_COMPOSER_COPY.needsLevels };
  }
  const index = Number(draft.target);
  const level =
    Number.isInteger(index) && index >= 0 && index < levels.length ? index : levels.length - 1;
  return {
    ok: true,
    question: {
      instructions,
      kind: { type: 'score', levels },
      threshold: p !== null ? kept({ type: 'at_least_level', level, p }) : null,
    },
  };
}

/** What the route stores for a question, compared to skip a save that would change nothing. */
const wireKey = (question: Question) => JSON.stringify(questionToWire(question));

/** How long typing pauses before the question is saved. */
export const DECISION_COMPOSER_SAVE_DELAY_MS = 500;

/** The longest a send waits for the question's save before it goes anyway. */
export const DECISION_QUESTION_SETTLE_MAX_MS = 3000;

/** Each mounted composer's "save now and wait", by chat. */
const settlers = new Map<string, () => Promise<void>>();

/**
 * R1-6: save the decision chat's question now, if one is typed and not yet stored,
 * and resolve once the save (or one already in flight) has answered, or after
 * {@link DECISION_QUESTION_SETTLE_MAX_MS}. A send in a decision chat awaits this
 * first: a question typed and then sent within the half-second the save waits for
 * (or while its PUT is still in flight) otherwise reached the daemon after the
 * message, which then had no question to answer. Resolves at once when no composer
 * for the chat is mounted. Never rejects.
 */
export function settleDecisionQuestion(sessionId: string | null | undefined): Promise<void> {
  const settle = sessionId ? settlers.get(sessionId) : undefined;
  if (!settle) return Promise.resolve();
  let timer: ReturnType<typeof setTimeout> | undefined;
  const cap = new Promise<void>((done) => {
    timer = setTimeout(done, DECISION_QUESTION_SETTLE_MAX_MS);
  });
  return Promise.race([settle().catch(() => undefined), cap]).finally(() => clearTimeout(timer));
}

type SaveState =
  | { status: 'loading' }
  | { status: 'idle' }
  | { status: 'saving' }
  | { status: 'saved' }
  | { status: 'refused'; message: string | null };

export interface DecisionComposerProps {
  /** The chat. Without one (a new chat not sent yet) there is nowhere to keep a question. */
  sessionId: string | null;
}

/**
 * What a chat whose main model is a decision model asks of every message: the
 * question, the kind of answer (yes or no, a choice, a score), and an optional
 * threshold. Drawn above the composer's text box; the text box then takes the
 * state to decide on, and Send sends it as the message.
 *
 * Read from and saved as the chat's `decision_chat.v1` through `GET` and
 * `PUT /sessions/{id}/decision-chat`, with the user-action proof. A save happens
 * half a second after typing pauses and at once when a field loses focus, so a
 * question typed before the text is pasted is stored before the message is sent.
 * A question that is not complete is not saved, and the status says what it
 * still needs (nothing while the question is empty: the placeholder says it).
 *
 * Essentials up front (owner rule, F-8): the question line, with its status at
 * the trailing end ("Saving…", "Saved"; a longer sentence takes its own line
 * under it, in the same `role="status"` node). The kind of answer, the options
 * or levels and the threshold sit behind "Question options", whose closed
 * summary says them in words ("Yes or no · when P(yes) is at or above 0.85").
 */
export function DecisionComposer({ sessionId }: DecisionComposerProps) {
  const [draft, setDraft] = React.useState<DecisionDraft>(emptyDraft);
  const [save, setSave] = React.useState<SaveState>({ status: sessionId ? 'loading' : 'idle' });
  const lastSaved = React.useRef<string | null>(null);
  const timer = React.useRef<number | null>(null);
  /** The save in flight, so a settle that finds nothing new to save still waits for it. */
  const inflight = React.useRef<Promise<void> | null>(null);
  const draftRef = React.useRef(draft);
  draftRef.current = draft;
  const loaded = save.status !== 'loading';
  const ids = React.useId();

  // The stored question, once per chat. Nothing is saved before it answers, so an
  // empty form can never overwrite a question the chat already has.
  React.useEffect(() => {
    if (!sessionId) return;
    const controller = new AbortController();
    setSave({ status: 'loading' });
    (async () => {
      let stored: Question | null = null;
      try {
        stored = await fetchDecisionChatQuestion(sessionId, controller.signal);
      } catch {
        // Unreadable: the fields start empty and the next complete question is saved.
      }
      if (controller.signal.aborted) return;
      lastSaved.current = stored ? wireKey(stored) : null;
      setDraft(draftFromQuestion(stored));
      setSave({ status: 'idle' });
    })();
    return () => controller.abort();
  }, [sessionId]);

  const flush = React.useCallback(async () => {
    if (timer.current !== null) {
      window.clearTimeout(timer.current);
      timer.current = null;
    }
    if (!sessionId) return;
    const verdict = questionFromDraft(draftRef.current);
    if (!verdict.ok) return;
    const json = wireKey(verdict.question);
    if (json === lastSaved.current) {
      // Stored, or being stored: wait for that save rather than start another.
      await inflight.current;
      return;
    }
    lastSaved.current = json;
    setSave({ status: 'saving' });
    const run = (async () => {
      try {
        const result = await saveDecisionChatQuestion(sessionId, verdict.question);
        if (result.ok) {
          setSave({ status: 'saved' });
        } else {
          lastSaved.current = null;
          setSave({ status: 'refused', message: result.message });
        }
      } catch {
        lastSaved.current = null;
        setSave({ status: 'refused', message: null });
      }
    })();
    inflight.current = run;
    try {
      await run;
    } finally {
      if (inflight.current === run) inflight.current = null;
    }
  }, [sessionId]);

  // A send awaits this chat's question (`settleDecisionQuestion`).
  React.useEffect(() => {
    if (!sessionId) return;
    const settle = () => flush();
    settlers.set(sessionId, settle);
    return () => {
      if (settlers.get(sessionId) === settle) settlers.delete(sessionId);
    };
  }, [sessionId, flush]);

  // A save waiting when the composer goes (the chat closes, the model changes)
  // still happens.
  React.useEffect(
    () => () => {
      if (timer.current !== null) void flush();
    },
    [flush]
  );

  const edit = (change: (current: DecisionDraft) => DecisionDraft) => {
    setDraft((current) => {
      const next = change(current);
      draftRef.current = next;
      return next;
    });
    if (!loaded) return;
    // An edit since the last answer: "saved" and a refusal no longer describe it.
    setSave((current) =>
      current.status === 'saved' || current.status === 'refused' ? { status: 'idle' } : current
    );
    if (timer.current !== null) window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => {
      timer.current = null;
      void flush();
    }, DECISION_COMPOSER_SAVE_DELAY_MS);
  };
  const onBlur = () => {
    if (loaded) void flush();
  };

  if (!sessionId) {
    return (
      <div className="br-decision-composer" data-testid="decision-composer" data-draft="">
        <p className="br-decision-composer__note">{DECISION_COMPOSER_COPY.draftNote}</p>
      </div>
    );
  }

  const verdict = questionFromDraft(draft);
  // The one thing the status says, and how: a short state at the end of the question
  // line, a longer sentence on its own line under it (the same node either way).
  const problem = !verdict.ok && draft.instructions.trim() !== '' ? verdict.problem : null;
  // Red is rare (design.md 9.14): a problem is said in the primary ink, never in a
  // status hue. A refused save is a failure, so it also draws the failure glyph; a
  // threshold out of range is an invalid field, whose field keeps the one red edge.
  const status: {
    text: string;
    wide: boolean;
    problem: boolean;
    glyph: 'check' | 'failed' | null;
  } =
    save.status === 'loading'
      ? { text: DECISION_COMPOSER_COPY.loading, wide: false, problem: false, glyph: null }
      : !verdict.ok
        ? {
            text: problem ?? '',
            wide: problem !== null,
            problem: problem === DECISION_COMPOSER_COPY.badThreshold,
            glyph: null,
          }
        : save.status === 'saving'
          ? { text: DECISION_COMPOSER_COPY.saving, wide: false, problem: false, glyph: null }
          : save.status === 'refused'
            ? {
                text: DECISION_COMPOSER_COPY.refused(save.message),
                wide: true,
                problem: true,
                glyph: 'failed',
              }
            : save.status === 'saved' || lastSaved.current === wireKey(verdict.question)
              ? { text: DECISION_COMPOSER_COPY.saved, wide: false, problem: false, glyph: 'check' }
              : { text: '', wide: false, problem: false, glyph: null };
  const thresholdInvalid =
    draft.threshold.trim() !== '' &&
    !verdict.ok &&
    verdict.problem === DECISION_COMPOSER_COPY.badThreshold;

  const targets =
    draft.kind === 'choice'
      ? draft.options
          .map((option) => option.text.trim())
          .filter(Boolean)
          .map((name) => ({ value: name, label: name }))
      : draft.kind === 'score'
        ? draft.levels.map((level, index) => ({
            value: String(index),
            label: DECISION_COMPOSER_COPY.atLeast(
              level.text.trim() || DECISION_COMPOSER_COPY.level(index + 1)
            ),
          }))
        : [];
  const target =
    draft.kind === 'choice'
      ? (targets.find((option) => option.value === draft.target) ?? targets[0] ?? null)
      : draft.kind === 'score'
        ? (targets.find((option) => option.value === draft.target) ??
          targets[targets.length - 1] ??
          null)
        : null;

  // The rule as words (never `≥` or `<`). A stored rule these fields cannot show
  // (below, or P(no)) is said as kept, so the person does not read the threshold as
  // P(yes) at or above it.
  const rule = verdict.ok ? verdict.question.threshold : null;
  const phrase = verdict.ok && rule ? ruleInWords(rule, verdict.question) : null;
  const kept = rule && (rule.below || rule.type === 'no') ? phrase : null;
  const summary = [
    draft.kind === 'choice'
      ? DECISION_COMPOSER_COPY.kindSummary.choice(draft.options.length)
      : draft.kind === 'score'
        ? DECISION_COMPOSER_COPY.kindSummary.score(draft.levels.length)
        : DECISION_COMPOSER_COPY.kindSummary.binary(),
    phrase,
  ]
    .filter(Boolean)
    .join(' · ');

  const entries = draft.kind === 'choice' ? draft.options : draft.levels;
  const listKey: 'options' | 'levels' = draft.kind === 'choice' ? 'options' : 'levels';
  const maxEntries = draft.kind === 'choice' ? 26 : 10;
  const minEntries = draft.kind === 'choice' ? 2 : 1;
  const statusId = `${ids}-status`;

  return (
    <div
      className="br-decision-composer"
      role="group"
      aria-label={DECISION_COMPOSER_COPY.region}
      data-testid="decision-composer"
    >
      <div className="br-decision-composer__line">
        <label htmlFor={`${ids}-question`} className="br-decision-composer__label">
          {DECISION_COMPOSER_COPY.question}
        </label>
        {/* Unfilled, like the composer's own text box: the question is written in the same
            card, in the same voice (a filled field read as a slab in dark mode, F-8). */}
        <input
          id={`${ids}-question`}
          type="text"
          className="br-decision-composer__question"
          data-testid="decision-composer-question"
          value={draft.instructions}
          placeholder={DECISION_COMPOSER_COPY.questionPlaceholder[draft.kind]}
          disabled={!loaded}
          aria-describedby={statusId}
          onChange={(event) => {
            const instructions = event.target.value;
            edit((current) => ({ ...current, instructions }));
          }}
          onBlur={onBlur}
        />
        <span
          id={statusId}
          className="br-decision-composer__status"
          role="status"
          data-state={save.status}
          data-wide={status.wide ? 'true' : undefined}
          data-tone={status.problem ? 'problem' : undefined}
          data-testid="decision-composer-status"
        >
          {status.glyph === 'check' ? <Check aria-hidden /> : null}
          {status.glyph === 'failed' ? (
            <AlertCircle aria-hidden data-testid="decision-composer-failed" />
          ) : null}
          {status.text}
        </span>
      </div>
      <Disclosure label={DECISION_COMPOSER_COPY.options} summary={summary}>
        <div
          className="br-decision-composer__options"
          data-testid="decision-composer-options-panel"
        >
          <div className="br-decision-composer__field">
            <span id={`${ids}-kind`} className="br-decision-composer__label">
              {DECISION_COMPOSER_COPY.kind}
            </span>
            <SegmentedControl<Kind>
              aria-labelledby={`${ids}-kind`}
              data-testid="decision-composer-kind"
              disabled={!loaded}
              value={draft.kind}
              onValueChange={(kind) => edit((current) => ({ ...current, kind, target: '' }))}
              options={[
                { value: 'binary', label: DECISION_COMPOSER_COPY.kinds.binary },
                { value: 'choice', label: DECISION_COMPOSER_COPY.kinds.choice },
                { value: 'score', label: DECISION_COMPOSER_COPY.kinds.score },
              ]}
            />
          </div>
          {draft.kind !== 'binary' && (
            <div
              className="br-decision-composer__entries"
              data-testid={`decision-composer-${listKey}`}
            >
              <span className="br-decision-composer__label">
                {draft.kind === 'choice'
                  ? DECISION_COMPOSER_COPY.optionsLabel
                  : DECISION_COMPOSER_COPY.levels}
              </span>
              <ul className="br-decision-composer__list">
                {entries.map((item, index) => {
                  const name =
                    draft.kind === 'choice'
                      ? DECISION_COMPOSER_COPY.option(index + 1)
                      : DECISION_COMPOSER_COPY.level(index + 1);
                  return (
                    <li key={item.id} className="br-decision-composer__entry">
                      <Input
                        aria-label={name}
                        placeholder={name}
                        value={item.text}
                        disabled={!loaded}
                        onChange={(event) => {
                          const value = event.target.value;
                          edit((current) => ({
                            ...current,
                            [listKey]: current[listKey].map((other) =>
                              other.id === item.id ? { ...other, text: value } : other
                            ),
                          }));
                        }}
                        onBlur={onBlur}
                      />
                      <Button
                        type="button"
                        variant="ghost"
                        size="xs"
                        shape="round"
                        disabled={!loaded || entries.length <= minEntries}
                        aria-label={
                          draft.kind === 'choice'
                            ? DECISION_COMPOSER_COPY.removeOption(item.text.trim() || name)
                            : DECISION_COMPOSER_COPY.removeLevel(item.text.trim() || name)
                        }
                        onClick={() =>
                          edit((current) => ({
                            ...current,
                            [listKey]: current[listKey].filter((other) => other.id !== item.id),
                          }))
                        }
                      >
                        <X aria-hidden />
                      </Button>
                    </li>
                  );
                })}
              </ul>
              <Button
                type="button"
                variant="ghost"
                size="xs"
                disabled={!loaded || entries.length >= maxEntries}
                onClick={() =>
                  edit((current) => ({ ...current, [listKey]: [...current[listKey], entry()] }))
                }
              >
                <Plus aria-hidden />
                {draft.kind === 'choice'
                  ? DECISION_COMPOSER_COPY.addOption
                  : DECISION_COMPOSER_COPY.addLevel}
              </Button>
            </div>
          )}
          <div className="br-decision-composer__field">
            <div className="br-decision-composer__threshold-row">
              {/* The InfoTip is the label's next sibling, never inside it. */}
              <span className="br-decision-composer__label-with-tip">
                <label htmlFor={`${ids}-threshold`} className="br-decision-composer__label">
                  {DECISION_COMPOSER_COPY.threshold}
                </label>
                <InfoTip
                  label={DECISION_COMPOSER_COPY.threshold}
                  help={DECISION_COMPOSER_COPY.thresholdInfo}
                />
              </span>
              <Input
                id={`${ids}-threshold`}
                machine
                data-testid="decision-composer-threshold"
                className="br-decision-composer__threshold"
                type="number"
                inputMode="decimal"
                min={0}
                max={1}
                step={0.05}
                value={draft.threshold}
                placeholder={DECISION_COMPOSER_COPY.thresholdPlaceholder}
                disabled={!loaded}
                aria-invalid={thresholdInvalid}
                aria-describedby={thresholdInvalid ? statusId : undefined}
                onChange={(event) => {
                  const threshold = event.target.value;
                  // Cleared, the stored rule goes with it: a new threshold is at or above, on yes.
                  edit((current) =>
                    threshold.trim() === ''
                      ? { ...current, threshold, below: false, binarySide: 'yes' }
                      : { ...current, threshold }
                  );
                }}
                onBlur={onBlur}
              />
              {draft.kind !== 'binary' && draft.threshold.trim() !== '' && targets.length > 0 && (
                <>
                  <span className="br-decision-composer__label" aria-hidden>
                    {DECISION_COMPOSER_COPY.thresholdOnWord}
                  </span>
                  <div className="br-decision-composer__target">
                    <Select
                      aria-label={DECISION_COMPOSER_COPY.thresholdOn}
                      options={targets}
                      value={target}
                      isSearchable={false}
                      isDisabled={!loaded}
                      menuPlacement="auto"
                      menuPortalTarget={typeof document === 'undefined' ? undefined : document.body}
                      onChange={(value: unknown) => {
                        const picked = value as { value: string } | null;
                        if (picked) edit((current) => ({ ...current, target: picked.value }));
                      }}
                      onBlur={onBlur}
                    />
                  </div>
                </>
              )}
            </div>
            {kept ? (
              <p className="br-decision-composer__note" data-testid="decision-composer-kept-rule">
                {DECISION_COMPOSER_COPY.keptRule(kept)}
              </p>
            ) : phrase ? (
              <p className="br-decision-composer__note" data-testid="decision-composer-meets">
                {DECISION_COMPOSER_COPY.meetsSentence(phrase)}
              </p>
            ) : null}
          </div>
        </div>
      </Disclosure>
    </div>
  );
}
