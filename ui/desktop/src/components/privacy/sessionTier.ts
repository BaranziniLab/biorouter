import type { SessionClassification } from '../../api/types.gen';

/**
 * Combining two readings of a chat's classification (issue #56, R10).
 *
 * # Why this is arithmetic and not a preference
 *
 * `SessionClassification` is a two-element lattice — `public < private` — and
 * the daemon reduces it with `max` over the life of a session
 * (`crates/biorouter/src/privacy/mod.rs`; CLAUDE.md calls it "a permanent
 * ratchet"). It rises on its own and falls only when the USER declassifies the
 * chat (§12.4, `privacy::declassify`). So:
 *
 * - A reading of `public` can become false at any moment, and says nothing
 *   about now. It is a lower bound, not an answer.
 * - A reading of `private` can become false only through a declassification,
 *   which every source here is told about (`sessionRowSync`, the change feed).
 *
 * ⚠ **That second line used to say "can never become false", and the tab strip
 * was built on it.** The live map in `ChatStreamRegistry` then refused to
 * follow its own store down, so a chat declassified with its tab open kept a
 * private tab icon until the renderer reloaded (defect D1, 2026-09-13). The
 * rule that survives is narrower, and it is two rules:
 *
 * - WITHIN one source, the newest reading wins. Each source is re-read when the
 *   row moves, in either direction, and holds what it last read.
 * - ACROSS sources, `max` — these functions. Two sources disagree only while
 *   one of them has not yet been re-read, and while that is so the chat is
 *   shown private. A merge that preferred "the fresher source" would let one
 *   that has not yet heard about a raise overwrite one that has, and that is
 *   the direction a badge must never be wrong in; the price of `max` is a
 *   lowering that shows up once the slower source has been re-read too.
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
 * absent from the result, and the glyph (`ChatKindIcon`) draws it as not yet
 * known. Asserting Public for a chat nobody has read is the same lie in the
 * other direction — and the glyph told exactly that lie, `tier ?? 'public'`,
 * until 2026-09-14, so every map here was right and the tab still said Public.
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
