import type { Session } from '../../api';

/**
 * The day-one notice's counting rules, and the one question the app asks of them: is it due.
 *
 * Moved here VERBATIM from `FirstRunPrivacyNotice.tsx`, which re-exports every name below so no
 * importer changed (FR-R4-4). The reason is where the boot reaches them: `renderer.tsx` starts the
 * count before the app has loaded (`firstRunNoticeDecision.ts`), and it must not pull the notice's
 * dialog, button and shell into the entry chunk to do it. This file imports a type from the API and
 * nothing else, so it carries no UI. The rules, the comments and the order are unchanged.
 */

/**
 * The numbers the day-one notice quotes, over the population **History actually
 * shows** (issue #56 §15.5).
 *
 * The mirror of `biorouter::session::session_manager::PrivacyNoticeCounts`, and
 * the two agree by construction rather than by coincidence: `GET /sessions`
 * without `include_subagents` is backed by `list_sessions_by_types(&[User,
 * Scheduled])`, whose `INNER JOIN messages` selects exactly the rows the Rust
 * helper's `EXISTS (SELECT 1 FROM messages …)` selects. Counting the array the
 * renderer already holds is therefore the same question asked of the same rows,
 * with no second endpoint to keep in step.
 */
export interface NoticeCounts {
  /** Private to the fail-closed reader, and visible in History. */
  privateVisible: number;
  /** Public, visible, and bound to a named provider — read, not guessed. */
  publicNamedVisible: number;
  /**
   * Public, visible, and bound to NO provider. The migration could not tell what
   * these ran on and failed **open**, by decision. Historically the largest of
   * the three, and the honest residual of the whole feature.
   */
  unknownProviderVisible: number;
  /** The denominator: everything History will list. */
  totalVisible: number;
  /**
   * Of the private rows, the ones the **migration** marked — a `backfill:*`
   * provenance rather than `turn:*`, `mcp:*` or `diverged:*`.
   *
   * ⚠ **This, not `privateVisible`, is what makes the notice due.** A fresh
   * install has no backfilled rows and never will; its private chats arrive one
   * at a time, from turns the user themselves ran on a private model. Triggering
   * on `privateVisible` would ambush that user weeks later with a modal
   * announcing that their chats "are now marked private because that is the
   * model each of them was last using" — describing a migration that never
   * touched their database. See {@link shouldShowFirstRunNotice}.
   */
  backfilledVisible: number;
  /**
   * The private rows grouped by the provider the migration read, descending by
   * count. This is what makes §15.5's "review by provider" list possible: a
   * backfill tier is a **guess the system made from the last-used provider**,
   * not a user's assertion about content, so the notice owes the user a way to
   * see the guess broken down before living with it.
   */
  privateByProvider: Array<{ provider: string; count: number }>;
}

/** A `privacy_reason` of `backfill:<provider>` yields `<provider>`. */
const BACKFILL_PREFIX = 'backfill:';

/** The bucket a private row with no readable provenance lands in. */
export const UNKNOWN_PROVIDER = 'unknown';

/**
 * Fail-closed, exactly like `SessionClassification::from_stored` and like
 * `PrivacyBadge`'s `Record` map: anything that is not precisely `'public'` reads
 * Private.
 *
 * That includes `undefined` — a daemon too old to send the column, or a
 * projection that dropped it. The alternative (treat unknown as public) would
 * make the notice quote a smaller, friendlier number than the badges beside it,
 * and the one property the notice must have is that it describes what the user
 * is about to see.
 */
function isPrivate(session: Session): boolean {
  return session.privacy_tier !== 'public';
}

function providerOf(session: Session): string {
  // The provenance first, because it records what the migration actually
  // concluded and survives a later provider switch; `provider_name` is the
  // fallback for a row raised by a turn rather than by the backfill.
  const reason = session.privacy_reason ?? '';
  if (reason.startsWith(BACKFILL_PREFIX)) {
    const named = reason.slice(BACKFILL_PREFIX.length).trim();
    if (named !== '') return named;
  }
  const bound = (session.provider_name ?? '').trim();
  return bound === '' ? UNKNOWN_PROVIDER : bound;
}

/**
 * Count the user's own conversations. Pure, and the only place the rule lives.
 *
 * ⚠ **Never hardcode these numbers.** §16's table was re-measured three times
 * while the feature was being designed and moved by a factor of three in four
 * days; the `user` NULL-provider bucket alone went from 29 rows to 2,831. A
 * notice carrying a figure from a design document is a notice that is wrong on
 * every machine including the author's — and this file's own comments were
 * caught quoting stale ones. Any figure written down in this folder must carry
 * the date it was measured, and nothing may read it.
 */
export function computeNoticeCounts(sessions: Session[]): NoticeCounts {
  let privateVisible = 0;
  let backfilledVisible = 0;
  let publicNamedVisible = 0;
  let unknownProviderVisible = 0;
  const byProvider = new Map<string, number>();

  for (const session of sessions) {
    if (isPrivate(session)) {
      privateVisible += 1;
      if ((session.privacy_reason ?? '').startsWith(BACKFILL_PREFIX)) backfilledVisible += 1;
      const provider = providerOf(session);
      byProvider.set(provider, (byProvider.get(provider) ?? 0) + 1);
      continue;
    }
    if ((session.provider_name ?? '').trim() === '') {
      unknownProviderVisible += 1;
    } else {
      publicNamedVisible += 1;
    }
  }

  const privateByProvider = [...byProvider.entries()]
    .map(([provider, count]) => ({ provider, count }))
    // Count descending, then name ascending — a stable order, so the list does
    // not reshuffle between two renders of the same data.
    .sort((a, b) => b.count - a.count || a.provider.localeCompare(b.provider));

  return {
    privateVisible,
    backfilledVisible,
    publicNamedVisible,
    unknownProviderVisible,
    totalVisible: sessions.length,
    privateByProvider,
  };
}

/**
 * Worth showing at all? Only if the **migration** marked something the user can
 * see.
 *
 * ⚠ **`backfilledVisible`, not `privateVisible`, and the difference is a real
 * defect either way round.** Every sentence in this notice is about a thing that
 * happened once, to data at rest, during an upgrade. On a machine where the
 * backfill marked nothing — a fresh install, or one whose chats all record a
 * commercial provider — those sentences describe nothing that occurred, and
 * `privateVisible` would still climb above zero the first time the user ran a
 * turn on Ollama. That fires this modal weeks after any upgrade, announcing a
 * migration that never touched their database.
 *
 * Exported so the surface that mounts the notice can decide **before** rendering
 * a modal, rather than opening one that says "0 conversations changed".
 */
export function shouldShowFirstRunNotice(counts: NoticeCounts): boolean {
  return counts.backfilledVisible > 0;
}
