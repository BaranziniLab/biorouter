import type { Snapshot } from '../crewApi';
import { nameKey } from '../identity';

/**
 * Starting an agent from the composer (design OW-10): `@agent`, `/agent` or `/task` as the first
 * word of the person's own draft starts a task in the channel, with the words after it as the task.
 *
 * Pure, and only ever asked of the draft in this composer: a message someone else posted, one the
 * person's agent wrote, or anything in the history never starts anything, because nothing else is
 * parsed. The daemon and the broker still admit every start.
 */

/** The trigger, then whitespace or the end. Nothing before it but whitespace. */
const LEADING = /^\s*(@agent|\/agent|\/task)(?=\s|$)/i;

export type AgentTrigger = '@agent' | '/agent' | '/task';

export interface AgentInvocation {
  trigger: AgentTrigger;
  /** The task: the draft without its trigger, trimmed. Empty for a trigger alone. */
  prompt: string;
}

/**
 * The invocation a draft makes, or `null` for a message.
 *
 * - **Leading only.** `hi @agent do x` is a message: a mention in a sentence never starts a task.
 * - **Whole word.** `@agentx`, `@agent.`, `/agent/foo`, `/tasks` and a pasted server path
 *   (`/pool1/data/x`) are messages.
 * - **Literal.** A hidden character inside the word (a zero-width space after `@a`) is a message,
 *   as the timeline draws it visibly.
 * - **A person called `agent`.** Usernames are server account names, so a member may be `agent`:
 *   then `@agent` mentions them, and `/agent` and `/task` still start a task (`agentNameTaken`).
 */
export function parseAgentInvocation(
  body: string,
  { agentNameTaken }: { agentNameTaken: boolean }
): AgentInvocation | null {
  const match = LEADING.exec(body);
  if (!match) return null;
  const trigger = match[1].toLowerCase() as AgentTrigger;
  if (trigger === '@agent' && agentNameTaken) return null;
  return { trigger, prompt: body.slice(match[0].length).trim() };
}

/**
 * Whether someone in the workspace, or someone who left it, has the username `agent` (compared by
 * name key, so `Agent` and a fullwidth `ａｇｅｎｔ` count): `@agent` then mentions them.
 */
export function agentNameTaken(snapshot: Snapshot | null | undefined): boolean {
  return [...(snapshot?.principals ?? []), ...(snapshot?.former_principals ?? [])].some(
    (person) => typeof person?.username === 'string' && nameKey(person.username) === 'agent'
  );
}

/**
 * What Enter does with a draft that starts a task, before Enter (the consent line says it):
 *
 * - `ready`: start the task at once, on the model the line names;
 * - `empty` (a trigger alone), `unknown-outcome` (the last start may have run: the pane's gate
 *   decides, C9 to C12) and `loading` (the model is not known yet): open Ask my agent with the
 *   words;
 * - `no-model` and `blocked` (the model would be refused here, `text` says why): open Ask my agent
 *   on its model picker;
 * - `files` and `too-long`: nothing starts, and the line says why. A task is words only, and the
 *   daemon takes at most {@link TASK_PROMPT_MAX_BYTES}.
 */
export type AgentStartState =
  | { kind: 'ready' }
  | { kind: 'empty' }
  | { kind: 'unknown-outcome' }
  | { kind: 'loading' }
  | { kind: 'no-model' }
  | { kind: 'blocked'; text: string }
  | { kind: 'files' }
  | { kind: 'too-long' };

export function agentStartState({
  invocation,
  hasFiles,
  unknownOutcome,
  modelsKnown,
  hasModel,
  blockText,
}: {
  invocation: AgentInvocation;
  /** The draft carries attachments or server paths. */
  hasFiles: boolean;
  unknownOutcome: boolean;
  /** The configured models have been read (or could not be, which is not "loading"). */
  modelsKnown: boolean;
  hasModel: boolean;
  /** Ask my agent's refusal of the model, before Start (`taskStartCheck`). */
  blockText: string | null;
}): AgentStartState {
  if (invocation.prompt === '') return { kind: 'empty' };
  if (hasFiles) return { kind: 'files' };
  if (taskBytes(invocation.prompt) > TASK_PROMPT_MAX_BYTES) return { kind: 'too-long' };
  if (unknownOutcome) return { kind: 'unknown-outcome' };
  if (!modelsKnown) return { kind: 'loading' };
  if (!hasModel) return { kind: 'no-model' };
  if (blockText) return { kind: 'blocked', text: blockText };
  return { kind: 'ready' };
}

/** The daemon takes a task of at most this many bytes of UTF-8 (`routes/crew.rs`). */
export const TASK_PROMPT_MAX_BYTES = 32_768;

/** The UTF-8 length of a task, as the daemon counts it. */
export function taskBytes(prompt: string): number {
  return new TextEncoder().encode(prompt).length;
}

/**
 * What the text before the caret is asking the suggestion list for, or `null`:
 *
 * - `leading`: the draft so far is one `@` or `/` word (`@`, `@ag`, `/t`), which offers the agent
 *   (and, after `@`, people);
 * - `person`: an `@` word after a space (`thanks @bo`), which offers people.
 *
 * `start` and `end` are the word's range in the draft, which accepting a suggestion replaces. A
 * word with anything but letters (`/pool1`, `/agent/`) asks for nothing, so the list closes at once
 * and never takes Enter from a path being typed.
 */
export type SuggestionQuery =
  | { kind: 'leading'; sigil: '@' | '/'; text: string; start: number; end: number }
  | { kind: 'person'; text: string; start: number; end: number };

const LEADING_WORD = /^(\s*)([@/])([A-Za-z]*)$/;
const PERSON_WORD = /(^|\s)@([A-Za-z0-9._-]*)$/;

export function suggestionQuery(body: string, caret: number): SuggestionQuery | null {
  const before = body.slice(0, Math.max(0, Math.min(caret, body.length)));
  // Only at the end of a word: a caret inside `@agent` (before `nt`) is not typing it.
  const after = body.slice(before.length);
  if (after !== '' && !/^\s/.test(after)) return null;
  const leading = LEADING_WORD.exec(before);
  if (leading) {
    const start = leading[1].length;
    return {
      kind: 'leading',
      sigil: leading[2] as '@' | '/',
      text: leading[3],
      start,
      end: before.length,
    };
  }
  const person = PERSON_WORD.exec(before);
  if (person) {
    const start = (person.index ?? 0) + person[1].length;
    return { kind: 'person', text: person[2], start, end: before.length };
  }
  return null;
}
