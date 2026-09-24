/**
 * The shapes the identity layer reads and produces.
 *
 * The INPUT types are deliberately minimal and local. They name only the fields
 * the display rule needs, every newer field optional, so that today's
 * `crewApi.ts` `Snapshot` and the S1-enriched one both satisfy them
 * structurally — and so this module never waits on an edit to `crewApi.ts`.
 * Anything the broker has not projected yet (former principals, projected
 * display names, `host_principal_id`, the daemon's collision labels) degrades
 * to the older rule instead of breaking.
 *
 * ⚠ The snapshot is forwarded by the daemon as untyped JSON, so every reader in
 * this directory also checks field types at runtime. A malformed principal is
 * dropped (and so renders as "Unknown member"), never echoed.
 */

/** A principal as the snapshot (or a message result's `people` map) projects it. */
export interface CrewPrincipalInput {
  id: string;
  username: string;
  /** Numeric account ID on the broker node. Used only for host detection; never displayed. */
  uid?: number | null;
  /** The self-set name. The display name defaults to it (naming design D2). */
  nickname?: string | null;
  /** S1: the broker's sanitized display name. Preferred over `nickname` when present. */
  display_name?: string | null;
  avatar?: string | null;
  /** `false` for a former (offboarded) member. Absent means active. */
  active?: boolean | null;
}

/** One entry of a message result's `people` map (S1), keyed by principal ID. */
export interface CrewPeopleMapEntry {
  username: string;
  display_name?: string | null;
  nickname?: string | null;
  avatar?: string | null;
  active?: boolean | null;
}

/** The fields of a workspace snapshot the people directory reads. */
export interface PeopleSnapshotInput {
  workspace: {
    host_uid?: number | null;
    /** S1: the host's principal, injected at projection time. Preferred over `host_uid`. */
    host_principal_id?: string | null;
  };
  actor: CrewPrincipalInput;
  principals?: readonly CrewPrincipalInput[] | null;
  /** S1: inactive principals the actor's visible objects still reference. */
  former_principals?: readonly CrewPrincipalInput[] | null;
}

/**
 * The daemon's per-principal label, projected on each observation `state` frame
 * (naming design, "The collision rule"). Only `collides` is read: it is computed
 * with the confusable skeleton, which the renderer cannot reproduce, whereas the
 * name itself already arrives as the principal's projected `display_name`.
 */
export interface DaemonPersonLabel {
  full?: string;
  short?: string;
  collides?: boolean;
}

export type DaemonPersonLabels = Readonly<Record<string, DaemonPersonLabel | null | undefined>>;

export type CrewPeopleMap = Readonly<Record<string, CrewPeopleMapEntry | null | undefined>>;

/**
 * A person, resolved and ready to display. Every string here is already
 * sanitized; nothing in it is an ID except `id`, which exists for keys and for
 * "Copy person ID" menus and is never rendered by this module.
 */
export interface CrewPerson {
  /** The principal ID, or `null` for someone who is not a principal yet (a joiner). */
  readonly id: string | null;
  /** The account username, without the `@`. */
  readonly username: string;
  /** The display name; equals `username` when the person has not chosen one. */
  readonly displayName: string;
  /** The full name on the server account (GECOS). Only a joiner at a host decision carries it. */
  readonly serverName: string | null;
  readonly avatar: string | null;
  readonly isFormer: boolean;
  readonly isHost: boolean;
  /** This person is the viewer. Rendering does not act on it unless asked (`you` prop/option). */
  readonly isYou: boolean;
  /** Another person in the workspace has a name that reads the same. */
  readonly collides: boolean;
}

/**
 * Where a person is shown, which decides how much of their identity is spelled
 * out (ui-redesign-spec, "Identity and naming display rules").
 *
 * - `header`: message heads, member rows, the You row. Display name, then `@username` as its own muted element.
 * - `inline`: notes, toasts, invitation rows, "Hosted by". `Display name (@username)`.
 * - `authority`: pickers, remove, ownership, offboard, consent. Always both, even when equal.
 * - `chip`: member stack, avatar tooltips. Display name, `@username` in a tooltip.
 * - `joiner`: a host deciding whether to admit someone. `@username` first, in mono.
 */
export type PersonContext = 'header' | 'inline' | 'authority' | 'chip' | 'joiner';

export const PERSON_CONTEXTS: readonly PersonContext[] = [
  'header',
  'inline',
  'authority',
  'chip',
  'joiner',
];

/** A person, or a principal ID to look up in a directory, or nothing known. */
export type PersonRef = CrewPerson | string | null | undefined;
