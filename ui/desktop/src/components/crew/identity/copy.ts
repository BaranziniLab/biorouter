/**
 * The identity layer's pinned strings (ui-redesign-spec, copy deck `msg.*`,
 * `members.*`, `invite.*` and "Identity and naming display rules").
 *
 * Other areas import these rather than retyping them, so a test that asserts
 * "Unknown member" and the component that renders it cannot drift apart.
 */
const AGENT_SUFFIX = "'s agent";

export const identityCopy = {
  /** Any principal the viewer has no projection for. Never replaced by an ID. */
  unknownMember: 'Unknown member',
  /** Appended after a former member's name, muted. */
  formerMember: 'former member',
  /** Appended after the viewer's own name when a surface asks for it. */
  you: 'you',
  /** The viewer's own agent. */
  yourAgent: 'Your agent',
  /** Follows a name to make it the name of that person's agent. */
  agentSuffix: AGENT_SUFFIX,
  /** Another person's agent; `name` is already formatted for its context. */
  agentOf: (name: string) => `${name}${AGENT_SUFFIX}`,
  /** Qualifies the full name a joiner's server account carries (GECOS). */
  serverAccountName: 'name on the server account',
  /** The separator between a name and its qualifiers. */
  separator: ' · ',
  /** A team whose stored name has nothing displayable left. */
  untitledTeam: 'Untitled team',
  /** A channel whose stored name has nothing displayable left (rendered as `#untitled`). */
  untitledChannel: 'untitled',
  /** A legacy workspace with no name, when the host is not known either. */
  unnamedWorkspace: 'Unnamed workspace',
  /** A legacy workspace with no name, labelled by its host. */
  hostsWorkspace: (host: string) => `${host}'s workspace`,
} as const;
