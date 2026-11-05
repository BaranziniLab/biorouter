import './decisionsChat.css';
import * as React from 'react';
import { Plus, X } from '../icons/app-icons';
import { Button } from '../ui/button';
import { Input } from '../ui/input';
import { SegmentedControl } from '../ui/segmented-control';
import { Select } from '../ui/Select';
import { questionToWire, type Question } from '../../utils/sessionDecisions';
import { fetchDecisionChatQuestion, saveDecisionChatQuestion } from './decisionChatApi';

export const DECISION_COMPOSER_COPY = {
  region: 'Decision question',
  question: 'Question',
  questionPlaceholder: 'Does this note state the patient has multiple sclerosis?',
  kind: 'Answer type',
  kinds: { binary: 'Yes or no', choice: 'Choice', score: 'Score' },
  options: 'Options',
  option: (n: number) => `Option ${n}`,
  removeOption: (name: string) => `Remove option ${name}`,
  addOption: 'Add option',
  levels: 'Levels, lowest first',
  level: (n: number) => `Level ${n}`,
  removeLevel: (name: string) => `Remove level ${name}`,
  addLevel: 'Add level',
  threshold: 'Threshold',
  thresholdPlaceholder: 'None',
  thresholdOn: 'Compared with',
  atLeast: (level: string) => `At least ${level}`,
  placeholder: 'Paste the text to decide on',
  draftNote:
    'Decision model: start your message with a question line such as “Q: Is the form signed?”, then the text to decide on.',
  saving: 'Saving the question…',
  saved: 'The question is saved for this chat.',
  loading: 'Reading this chat’s question…',
  needsQuestion: 'Write the question to ask of each message.',
  needsOptions: 'A choice needs 2 to 26 options, each with its own name.',
  needsLevels: 'A score needs 1 to 10 levels, each with a label.',
  badThreshold: 'The threshold is a probability from 0 to 1.',
  refused: (message: string | null) =>
    message ??
    'Biorouter could not save the question. A “Q:” question line at the start of your message still works.',
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
  if (draft.kind === 'binary') {
    return {
      ok: true,
      question: {
        instructions,
        kind: { type: 'binary' },
        threshold: p !== null ? { type: 'yes', p } : null,
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
        threshold: p !== null ? { type: 'option', name: target, p } : null,
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
      threshold: p !== null ? { type: 'at_least_level', level, p } : null,
    },
  };
}

/** What the route stores for a question, compared to skip a save that would change nothing. */
const wireKey = (question: Question) => JSON.stringify(questionToWire(question));

/** How long typing pauses before the question is saved. */
export const DECISION_COMPOSER_SAVE_DELAY_MS = 500;

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
 * A question that is not complete is not saved, and the line under the fields
 * says what it still needs.
 */
export function DecisionComposer({ sessionId }: DecisionComposerProps) {
  const [draft, setDraft] = React.useState<DecisionDraft>(emptyDraft);
  const [save, setSave] = React.useState<SaveState>({ status: sessionId ? 'loading' : 'idle' });
  const lastSaved = React.useRef<string | null>(null);
  const timer = React.useRef<number | null>(null);
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
    if (json === lastSaved.current) return;
    lastSaved.current = json;
    setSave({ status: 'saving' });
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
  }, [sessionId]);

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
        <p className="br-decision-composer__status">{DECISION_COMPOSER_COPY.draftNote}</p>
      </div>
    );
  }

  const verdict = questionFromDraft(draft);
  const status =
    save.status === 'loading'
      ? DECISION_COMPOSER_COPY.loading
      : !verdict.ok
        ? verdict.problem
        : save.status === 'saving'
          ? DECISION_COMPOSER_COPY.saving
          : save.status === 'refused'
            ? DECISION_COMPOSER_COPY.refused(save.message)
            : save.status === 'saved' || lastSaved.current === wireKey(verdict.question)
              ? DECISION_COMPOSER_COPY.saved
              : '';

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

  const entries = draft.kind === 'choice' ? draft.options : draft.levels;
  const listKey: 'options' | 'levels' = draft.kind === 'choice' ? 'options' : 'levels';
  const maxEntries = draft.kind === 'choice' ? 26 : 10;
  const minEntries = draft.kind === 'choice' ? 2 : 1;

  return (
    <div
      className="br-decision-composer"
      role="group"
      aria-label={DECISION_COMPOSER_COPY.region}
      data-testid="decision-composer"
    >
      <div className="br-decision-composer__row">
        <label htmlFor={`${ids}-question`} className="br-decision-composer__label">
          {DECISION_COMPOSER_COPY.question}
        </label>
        <Input
          id={`${ids}-question`}
          data-testid="decision-composer-question"
          value={draft.instructions}
          placeholder={DECISION_COMPOSER_COPY.questionPlaceholder}
          disabled={!loaded}
          onChange={(event) => {
            const instructions = event.target.value;
            edit((current) => ({ ...current, instructions }));
          }}
          onBlur={onBlur}
        />
      </div>
      <div className="br-decision-composer__row br-decision-composer__row--controls">
        <SegmentedControl<Kind>
          aria-label={DECISION_COMPOSER_COPY.kind}
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
        <label htmlFor={`${ids}-threshold`} className="br-decision-composer__label">
          {DECISION_COMPOSER_COPY.threshold}
        </label>
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
          aria-invalid={
            draft.threshold.trim() !== '' &&
            !verdict.ok &&
            verdict.problem === DECISION_COMPOSER_COPY.badThreshold
          }
          onChange={(event) => {
            const threshold = event.target.value;
            edit((current) => ({ ...current, threshold }));
          }}
          onBlur={onBlur}
        />
        {draft.kind !== 'binary' && draft.threshold.trim() !== '' && targets.length > 0 && (
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
        )}
      </div>
      {draft.kind !== 'binary' && (
        <div className="br-decision-composer__entries" data-testid={`decision-composer-${listKey}`}>
          <span className="br-decision-composer__label">
            {draft.kind === 'choice'
              ? DECISION_COMPOSER_COPY.options
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
      <p
        className="br-decision-composer__status"
        role="status"
        data-state={save.status}
        data-testid="decision-composer-status"
      >
        {status}
      </p>
    </div>
  );
}
