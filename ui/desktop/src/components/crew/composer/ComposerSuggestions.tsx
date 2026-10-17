import { useEffect, useState, type KeyboardEvent, type ReactNode } from 'react';
import type { Channel, Snapshot } from '../crewApi';
import { displayNameKey, nameKey, personLabel, type PeopleDirectory } from '../identity';
import { suggestionQuery, type SuggestionQuery } from './agentInvocation';
import { composerCopy } from './copy';

/** One row of the composer's suggestion list. */
export interface ComposerSuggestion {
  /** Stable within one list: the row's DOM id is built from it. */
  key: string;
  kind: 'agent' | 'person';
  /** The row's first line: `@agent`, `/task`, or a person in the authority form. */
  label: string;
  /** The second line, when there is one. */
  detail?: ReactNode;
  /** What replaces the word being typed when the row is accepted, trailing space included. */
  insert: string;
}

/** How many people the list offers at most; typing more narrows it. */
export const SUGGESTED_PEOPLE_MAX = 8;

const AGENT_COMMANDS = ['agent', 'task'] as const;

/**
 * The rows for `query`: the agent first (a leading `@` or `/` word only), then the channel's
 * members.
 *
 * - **The agent row** is `@agent` ("Your agent · {model} · posts in #general") after a leading `@`,
 *   and `/agent` and `/task` after a leading `/`. Its name is never "Ask my agent", which stays the
 *   composer button's alone (ui-redesign-spec, C6). When someone is called `agent`, the `@` row
 *   says so and inserts `/agent` instead, since `@agent` then mentions them.
 * - **People** are the channel's members, named in the authority form ("Bob Lee (@bob)"), so a
 *   nickname that imitates another person's cannot pass for them (naming design, rule 3). A row
 *   inserts `@username`. They match by username or by display name, from their first letters.
 *
 * Accepting a row inserts text and nothing else: Enter starts a task only once the list is closed.
 */
export function buildSuggestions({
  query,
  snapshot,
  channel,
  slug,
  dir,
  agentNameTaken,
  agentDetail,
}: {
  query: SuggestionQuery;
  snapshot: Snapshot | null;
  channel: Channel | null;
  /** The channel's slug, without the `#`, as the composer names it. */
  slug: string;
  dir: PeopleDirectory | null;
  agentNameTaken: boolean;
  /** The agent row's second line ("Your agent · GPT-5.5 · posts in #general"). */
  agentDetail: ReactNode;
}): ComposerSuggestion[] {
  if (!channel) return [];
  const rows: ComposerSuggestion[] = [];
  const typed = query.text.toLowerCase();
  // A trigger typed out in full offers nothing more for it: Enter then does what the consent line
  // says (a trigger alone opens Ask my agent) rather than inserting the word it already is.
  if (
    query.kind === 'leading' &&
    query.sigil === '@' &&
    'agent'.startsWith(typed) &&
    (typed !== 'agent' || agentNameTaken)
  ) {
    const named = agentNameTaken
      ? snapshot?.principals.find((person) => nameKey(person.username) === 'agent')
      : undefined;
    rows.push(
      agentNameTaken
        ? {
            key: 'agent',
            kind: 'agent',
            label: '/agent',
            detail: composerCopy.agentNameTaken(
              named ? personLabel(named.id, 'authority', dir) : '@agent'
            ),
            insert: '/agent ',
          }
        : { key: 'agent', kind: 'agent', label: '@agent', detail: agentDetail, insert: '@agent ' }
    );
  }
  if (query.kind === 'leading' && query.sigil === '/') {
    for (const command of AGENT_COMMANDS) {
      if (!command.startsWith(typed) || command === typed) continue;
      rows.push({
        key: command,
        kind: 'agent',
        label: `/${command}`,
        detail: composerCopy.slashRowDetail(slug),
        insert: `/${command} `,
      });
    }
  }
  if (query.kind === 'person' || query.sigil === '@') {
    const members = new Set(channel.members);
    const people = (snapshot?.principals ?? []).filter(
      (person) =>
        members.has(person.id) &&
        person.active !== false &&
        typeof person.username === 'string' &&
        person.username !== '' &&
        (typed === '' ||
          person.username.toLowerCase().startsWith(typed) ||
          displayNameKey(person.display_name || person.nickname || '').startsWith(nameKey(typed)))
    );
    for (const person of people.slice(0, SUGGESTED_PEOPLE_MAX)) {
      rows.push({
        key: `person:${person.id}`,
        kind: 'person',
        label: personLabel(person.id, 'authority', dir),
        insert: `@${person.username} `,
      });
    }
  }
  return rows;
}

/** The draft and caret after accepting `row` for `query`. */
export function acceptSuggestion(
  body: string,
  query: SuggestionQuery,
  row: ComposerSuggestion
): { body: string; caret: number } {
  const rest = body.slice(query.end);
  // The row ends in a space; a space already after the word is not doubled.
  const tail = row.insert.endsWith(' ') && /^[ \t]/.test(rest) ? rest.slice(1) : rest;
  return {
    body: `${body.slice(0, query.start)}${row.insert}${tail}`,
    caret: query.start + row.insert.length,
  };
}

export interface ComposerSuggestionsState {
  /** The rows on show; empty while the list is closed. */
  rows: ComposerSuggestion[];
  open: boolean;
  active: number;
  setActive(index: number): void;
  /** Accept row `index`: the draft and caret it makes, or `null` when there is no such row. */
  accept(index: number): { body: string; caret: number } | null;
  /**
   * The textarea's keys while the list is open: ArrowUp/Down move, Enter and Tab accept (`accept`:
   * the caller applies `accept(active)`), Escape closes. `true` for another key the list took,
   * `false` for one the textarea handles as before. Nothing while an input method is composing
   * (C15), so the Enter that commits a composition never accepts a row.
   */
  onKeyDown(event: KeyboardEvent<HTMLTextAreaElement>): 'accept' | boolean;
}

/**
 * The suggestion list's state for the text before the caret. It opens on a leading `@` or `/` word
 * and on an `@` word after a space, and closes when nothing matches, on Escape (until the word
 * changes), and while the textarea does not have focus.
 */
export function useComposerSuggestions({
  body,
  caret,
  focused,
  build,
}: {
  body: string;
  caret: number;
  focused: boolean;
  build(query: SuggestionQuery): ComposerSuggestion[];
}): ComposerSuggestionsState {
  const query = focused ? suggestionQuery(body, caret) : null;
  const word = query ? `${query.start}:${body.slice(query.start, query.end)}` : null;
  const [dismissed, setDismissed] = useState<string | null>(null);
  const rows = query && word !== dismissed ? build(query) : [];
  const open = rows.length > 0;
  const [active, setActive] = useState(0);
  // A new word starts at the first row; Escape lasts until the word changes.
  useEffect(() => {
    setActive(0);
    setDismissed(null);
  }, [word]);
  const shown = Math.min(active, Math.max(0, rows.length - 1));

  const accept = (index: number) => {
    const row = rows[index];
    return query && row ? acceptSuggestion(body, query, row) : null;
  };
  const onKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>): 'accept' | boolean => {
    if (!open || event.nativeEvent.isComposing || event.nativeEvent.keyCode === 229) return false;
    const plain = !event.shiftKey && !event.altKey && !event.ctrlKey && !event.metaKey;
    switch (event.key) {
      case 'ArrowDown':
        event.preventDefault();
        setActive((shown + 1) % rows.length);
        return true;
      case 'ArrowUp':
        event.preventDefault();
        setActive((shown - 1 + rows.length) % rows.length);
        return true;
      case 'Escape':
        event.preventDefault();
        event.stopPropagation();
        setDismissed(word);
        return true;
      case 'Enter':
      case 'Tab':
        if (!plain) return false;
        event.preventDefault();
        return 'accept';
      default:
        return false;
    }
  };
  return { rows, open, active: shown, setActive, accept, onKeyDown };
}

/**
 * The list itself, above the text inside the card. A `listbox` the textarea points to
 * (`aria-controls`, `aria-activedescendant`); focus stays in the textarea, so a pointer press on a
 * row does not take it (`onMouseDown` is prevented) and a click accepts the row.
 */
export function ComposerSuggestions({
  id,
  rows,
  active,
  onHover,
  onPick,
}: {
  id: string;
  rows: readonly ComposerSuggestion[];
  active: number;
  onHover(index: number): void;
  onPick(index: number): void;
}) {
  return (
    <div
      className="crew-compose-suggestions"
      role="listbox"
      id={id}
      aria-label={composerCopy.suggestions}
    >
      {rows.map((row, index) => (
        <div
          key={row.key}
          id={suggestionId(id, index)}
          role="option"
          aria-selected={index === active}
          data-active={index === active ? 'true' : undefined}
          data-kind={row.kind}
          className="crew-compose-suggestion"
          onMouseDown={(event) => event.preventDefault()}
          onMouseMove={() => {
            if (index !== active) onHover(index);
          }}
          onClick={() => onPick(index)}
        >
          <span className="crew-compose-suggestion-label">{row.label}</span>
          {row.detail ? <span className="crew-compose-suggestion-detail">{row.detail}</span> : null}
        </div>
      ))}
    </div>
  );
}

/** The DOM id of row `index` of list `id`, for `aria-activedescendant`. */
export function suggestionId(id: string, index: number): string {
  return `${id}-option-${index}`;
}
