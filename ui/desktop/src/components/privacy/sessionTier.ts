import type { SessionClassification } from '../../api/types.gen';

/**
 * Combining two readings of a chat's classification (issue #56, R10).
 *
 * # Why this is arithmetic and not a preference
 *
 * `SessionClassification` is a two-element lattice — `public < private` — and
 * the daemon reduces it with `max` over the life of a session
 * (`crates/biorouter/src/privacy/mod.rs`; CLAUDE.md calls it "a permanent
 * ratchet"). It only ever rises. That single fact decides every question a
 * caller could otherwise get wrong:
 *
 * - A reading of `private` can never become false. Whoever saw it saw a fact
 *   about the row that still holds, however old the reading is.
 * - A reading of `public` can become false at any moment, and says nothing
 *   about now. It is a lower bound, not an answer.
 *
 * So two readings of the same chat are combined with `max`, never with
 * "whichever is fresher". Freshness is not the ordering that matters here, and
 * a merge that preferred the newer source would let a source which has not yet
 * heard about a ratchet overwrite one that has.
 *
 * # The direction a mistake must fall in
 *
 * A chat that IS private and is drawn unmarked is the unsafe failure: the user
 * is looking at a surface that quietly under-states what the chat holds. A chat
 * drawn private when it is not would be merely wrong. `max` makes the first
 * impossible from any source that has ever seen the truth, and the second
 * impossible outright — because no source in this renderer invents `private`,
 * they only ever report a row.
 *
 * ⚠ **`undefined` is not `public`.** An id no source has an opinion about stays
 * absent from the result, and the glyph renders unmarked. Asserting Public for
 * a chat nobody has read is the same lie in the other direction.
 */

/** `max` over `public < private`; `undefined` is "no opinion", not `public`. */
export function raiseTier(
  current: SessionClassification | undefined,
  incoming: SessionClassification | undefined
): SessionClassification | undefined {
  if (current === 'private' || incoming === 'private') return 'private';
  if (current === 'public' || incoming === 'public') return 'public';
  return undefined;
}

/**
 * Fold any number of per-session tier maps into one, per id, with
 * {@link raiseTier}.
 *
 * Written variadic on purpose: the tab strip merges a LIVE map (what the chat
 * stores in this window hold right now) over a CACHED one (the session list),
 * and the whole point is that neither is privileged — adding a third source
 * must not need a new rule about which of the three wins.
 */
export function mergeSessionTiers(
  ...sources: ReadonlyArray<Readonly<Record<string, SessionClassification>> | undefined | null>
): Record<string, SessionClassification> {
  const merged: Record<string, SessionClassification> = {};
  for (const source of sources) {
    if (!source) continue;
    for (const [sessionId, tier] of Object.entries(source)) {
      const raised = raiseTier(merged[sessionId], tier);
      if (raised) merged[sessionId] = raised;
    }
  }
  return merged;
}

/**
 * True when `next` says something `previous` does not — the test a caller uses
 * to decide whether to publish a new map at all.
 *
 * Identity-stable output matters more here than it looks: these maps are read
 * through `useSyncExternalStore` and passed as a prop to every tab strip in
 * every pane, so a new object per notification re-renders all of them once per
 * streamed token to discover that nothing moved (#22).
 */
export function sessionTiersDiffer(
  previous: Readonly<Record<string, SessionClassification>>,
  next: Readonly<Record<string, SessionClassification>>
): boolean {
  const previousIds = Object.keys(previous);
  if (previousIds.length !== Object.keys(next).length) return true;
  return previousIds.some((id) => previous[id] !== next[id]);
}
