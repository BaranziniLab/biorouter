import { identityCopy } from './copy';
import {
  displayNameIsUsername,
  isolate,
  personDisplayName,
  sanitizeAvatarText,
  sanitizeUsername,
  usableName,
} from './displayText';
import type { PeopleDirectory } from './usePeopleDirectory';
import type { CrewPeopleMapEntry, CrewPerson, PersonContext, PersonRef } from './types';

/**
 * The display rule (ui-redesign-spec, "Identity and naming display rules";
 * naming-design.md, "The display rule"), decided once in {@link personLayout}
 * and rendered twice: as elements by `PersonName`, and as a plain string by
 * {@link personLabel} for the places a tree cannot go — an `aria-label`, a
 * confirmation title, a toast.
 *
 * Nothing here ever returns an ID. A principal the directory does not know is
 * "Unknown member", and a string passed as the person is only ever used to look
 * someone up.
 */

export interface PersonLabelOptions {
  /** Name the person's agent instead: "{name}'s agent", or "Your agent". */
  agent?: boolean;
  /**
   * The person is the viewer. Their agent becomes "Your agent" (except at an
   * authority point, which always names the owner in full); the person
   * themselves gains " · you".
   */
  you?: boolean;
}

/**
 * Resolves a reference against a directory. A string is a principal ID and is
 * looked up, never returned. A person carrying an ID is refreshed from the
 * directory when it knows them, so its flags (former, collides) are current.
 * `null` means "Unknown member".
 */
export function resolvePerson(person: PersonRef, dir?: PeopleDirectory | null): CrewPerson | null {
  if (typeof person === 'string') return dir?.byId(person) ?? null;
  if (!person || typeof person !== 'object') return null;
  const known = person.id !== null && dir ? dir.byId(person.id) : null;
  const resolved = known ?? person;
  return sanitizeUsername(resolved.username) ? resolved : null;
}

/** Where `@username` goes when something else leads. */
export type HandlePlacement =
  /** Not shown (an agent's inline label without a collision). */
  | 'none'
  /** Its own muted element after the name (header). */
  | 'secondary'
  /** `Name (@username)`. */
  | 'paren'
  /** In a tooltip, and in visually hidden text for assistive technology (chip). */
  | 'tooltip';

/** Everything a rendering of one person is made of. */
export type PersonLayout =
  | { kind: 'unknown'; agent: boolean }
  | { kind: 'joiner'; handle: string; serverName: string | null }
  | {
      kind: 'person';
      /** What leads: the display name, the handle standing in for it, or "Your agent". */
      lead: 'name' | 'handle' | 'your-agent';
      displayName: string;
      handle: string;
      handlePlacement: HandlePlacement;
      /** Follow the lead (and a parenthesized handle) with "'s agent". */
      agentOf: boolean;
      former: boolean;
      youSuffix: boolean;
    };

/**
 * Decides what a person looks like in a context.
 *
 * | context | person | agent |
 * |---|---|---|
 * | `header` | **Bob Lee** `@bob`; **@bob** when the names are equal | **Bob Lee's agent** `@bob`; **Your agent** `@bob` |
 * | `inline` | Bob Lee (@bob); @bob when equal | Bob Lee's agent; Your agent |
 * | `authority` | always Bob Lee (@bob), even bob (@bob) | Bob Lee (@bob)'s agent, even for the viewer |
 * | `chip` | Bob Lee, `@bob` in a tooltip; @bob when equal | Bob Lee's agent |
 * | `joiner` | `@bob` · Bob Lee (name on the server account) | — |
 *
 * On a collision every context spells the handle out: Sam Park (@spark), and
 * Sam Park (@spark)'s agent. A former member gains " · former member".
 */
export function personLayout(
  person: CrewPerson | null,
  context: PersonContext,
  options: PersonLabelOptions = {}
): PersonLayout {
  const agent = options.agent === true;
  const username = person ? sanitizeUsername(person.username) : '';
  if (!person || !username) return { kind: 'unknown', agent };

  const handle = `@${username}`;
  if (context === 'joiner') {
    return { kind: 'joiner', handle, serverName: usableName(person.serverName) || null };
  }

  const displayName = personDisplayName(person.displayName, username);
  const equal = displayNameIsUsername(displayName, username);
  const collides = person.collides === true;
  const former = person.isFormer === true;
  const common = { kind: 'person' as const, displayName, handle, former };

  if (agent && options.you === true && context !== 'authority') {
    return {
      ...common,
      lead: 'your-agent',
      handlePlacement: context === 'header' ? 'secondary' : 'none',
      agentOf: false,
      youSuffix: false,
    };
  }

  const youSuffix = !agent && options.you === true;
  const shape = { ...common, agentOf: agent, youSuffix };
  switch (context) {
    case 'authority':
      return { ...shape, lead: 'name', handlePlacement: 'paren' };
    case 'header':
      return equal
        ? { ...shape, lead: 'handle', handlePlacement: 'none' }
        : { ...shape, lead: 'name', handlePlacement: 'secondary' };
    case 'inline':
      if (equal) return { ...shape, lead: 'handle', handlePlacement: 'none' };
      return { ...shape, lead: 'name', handlePlacement: agent && !collides ? 'none' : 'paren' };
    case 'chip':
      if (equal) return { ...shape, lead: 'handle', handlePlacement: 'none' };
      return { ...shape, lead: 'name', handlePlacement: collides ? 'paren' : 'tooltip' };
  }
}

/**
 * The display rule as one string, for accessible names, confirmations and
 * toasts. It reads exactly as the rendered `PersonName` does, except that a
 * header's separate `@username` element becomes `(@username)`, since an
 * accessible name needs both parts: `Bob Lee (@bob)`, `Bob Lee's agent (@bob)`.
 * A chip's tooltip-only handle is left out, as it is from the chip's visible
 * text — use `inline` for an accessible name.
 *
 * A name containing right-to-left text is wrapped in Unicode isolates
 * (U+2068 … U+2069) so it cannot reorder the rest of the string.
 */
export function personLabel(
  person: PersonRef,
  context: PersonContext,
  dir?: PeopleDirectory | null,
  options: PersonLabelOptions = {}
): string {
  const layout = personLayout(resolvePerson(person, dir), context, options);

  if (layout.kind === 'unknown') {
    return layout.agent
      ? identityCopy.agentOf(identityCopy.unknownMember)
      : identityCopy.unknownMember;
  }
  if (layout.kind === 'joiner') {
    const handle = isolate(layout.handle);
    return layout.serverName
      ? `${handle}${identityCopy.separator}${isolate(layout.serverName)} (${identityCopy.serverAccountName})`
      : handle;
  }

  const handle = isolate(layout.handle);
  let text =
    layout.lead === 'your-agent'
      ? identityCopy.yourAgent
      : layout.lead === 'handle'
        ? handle
        : isolate(layout.displayName);
  if (layout.handlePlacement === 'paren') text = `${text} (${handle})`;
  if (layout.agentOf) text = identityCopy.agentOf(text);
  if (layout.handlePlacement === 'secondary') text = `${text} (${handle})`;
  if (layout.former) text += `${identityCopy.separator}${identityCopy.formerMember}`;
  if (layout.youSuffix) text += `${identityCopy.separator}${identityCopy.you}`;
  return text;
}

/** Shorthand for `personLabel(person, context, dir, { ...options, agent: true })`. */
export function agentLabel(
  person: PersonRef,
  context: Exclude<PersonContext, 'joiner'>,
  dir?: PeopleDirectory | null,
  options: Omit<PersonLabelOptions, 'agent'> = {}
): string {
  return personLabel(person, context, dir, { ...options, agent: true });
}

/**
 * A person who is not a principal yet: a joiner waiting for the host's decision,
 * or an invitation result. `serverName` is the full name on the server account,
 * shown only as "(name on the server account)" and never as the person's display
 * name — it is offered to them, never applied silently (naming design D2).
 */
export function joinerPerson(username: string, serverName?: string | null): CrewPerson {
  const handle = sanitizeUsername(username);
  return Object.freeze({
    id: null,
    username: handle,
    displayName: handle,
    serverName: usableName(serverName) || null,
    avatar: null,
    isFormer: false,
    isHost: false,
    isYou: false,
    collides: false,
  });
}

/**
 * A person projected on its own, outside any directory — for example an
 * invitation's projected inviter (`{username, display_name}`). Prefer
 * `directory.byId` whenever the person can be in the snapshot: only the
 * directory knows about collisions and about names that imitate someone else's
 * username. `null` (rendered "Unknown member") when there is no usable username.
 */
export function personFromProjection(
  projection: (CrewPeopleMapEntry & { id?: string | null }) | null | undefined,
  flags: { isYou?: boolean; isHost?: boolean; collides?: boolean } = {}
): CrewPerson | null {
  if (!projection || typeof projection !== 'object') return null;
  const username = sanitizeUsername(projection.username);
  if (!username) return null;
  const name =
    typeof projection.display_name === 'string' && projection.display_name
      ? projection.display_name
      : projection.nickname;
  return Object.freeze({
    id: typeof projection.id === 'string' && projection.id ? projection.id : null,
    username,
    displayName: personDisplayName(name, username),
    serverName: null,
    avatar: sanitizeAvatarText(projection.avatar) || null,
    isFormer: projection.active === false,
    isHost: flags.isHost === true,
    isYou: flags.isYou === true,
    collides: flags.collides === true,
  });
}
