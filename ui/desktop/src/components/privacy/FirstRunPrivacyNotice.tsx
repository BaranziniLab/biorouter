import { useEffect, useState } from 'react';
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from '../ui/dialog';
import { Button } from '../ui/button';
import { listSessions, type Session } from '../../api';
import { userActionHeaders } from '../../utils/userAction';

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
 * How a person names each private-tier provider. Unknown ids pass through
 * unchanged rather than being dropped: a provider this build has never heard of
 * still marked the user's chats, and hiding it would leave a row in the review
 * list the user cannot account for.
 */
const PROVIDER_LABELS: Record<string, string> = {
  versa_azure: 'Versa (Azure)',
  versa_bedrock: 'Versa (Bedrock)',
  llamacpp: 'Llama Server',
  ollama: 'Ollama',
  [UNKNOWN_PROVIDER]: 'Provider not recorded',
};

export function providerLabel(provider: string): string {
  return PROVIDER_LABELS[provider] ?? provider;
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

/**
 * Read the History population the notice describes.
 *
 * ⚠ **Not `refreshSessionList`, and not for want of a cache.** That module's own
 * comment says its `includeSubagents` argument is part of the cache *identity*
 * and that a second consumer passing one "would invalidate History's toggle and
 * silently drop the children" — which is precisely what `refreshSessionList(false)`
 * did from here: opening the notice reset a user who had subagents shown. The
 * keyless call is no better, because it sends whatever identity the cache is
 * holding, and if that is `true` the notice silently counts subagent sessions
 * that History is not showing — the exact over-count this whole type exists to
 * avoid. So the notice asks the daemon the one question it means, owns the
 * answer, and touches no shared state. It costs one GET, once per install.
 */
async function fetchVisibleSessions(): Promise<Session[]> {
  // With the user's proof: without it the daemon omits every private chat —
  // the chats this notice exists to count (issue #56, QA 2026-09-10 M1).
  const response = await listSessions<true>({
    throwOnError: true,
    headers: await userActionHeaders(),
    query: { include_subagents: false },
  });
  return response.data.sessions;
}

export interface FirstRunPrivacyNoticeProps {
  open: boolean;
  /** Acknowledged. The caller owns remembering that it was. */
  onDismiss: () => void;
  /**
   * Counts to render instead of fetching. For a caller that already holds the
   * session list (and for tests). When absent the notice reads the user's own
   * database through the same cache History does.
   */
  counts?: NoticeCounts;
  /**
   * §13.5's day-one extension disclosure: the **enabled**, **Public** extensions
   * that declare clinical-looking credentials, by name. Empty on a machine where
   * there are none, which hides the paragraph entirely.
   *
   * Passed in rather than computed here because the gate already holds the
   * extension list, and because a notice that renders nothing until a second
   * fetch lands would show its counts and then grow a paragraph under the user's
   * cursor.
   */
  publicClinicalExtensions?: string[];
}

/**
 * The day-one notice (issue #56 §15.5).
 *
 * ⚠ **It exists because the alternative is a week of unexplained refusals.** The
 * backfill marks a large fraction of an established user's history private in
 * one step — on the machine this was measured against on 2026-08-03, 654 of the
 * 4,034 conversations History shows, out of 1,486 rows raised in the database.
 * Every one of those chats then refuses the commercial model its owner normally
 * reaches for, with a modal that explains the rule but not why *this* chat is
 * subject to it. One screen of "here is what just changed, and here is the
 * number" is the whole mitigation. (Those figures are a dated measurement and
 * nothing reads them — see {@link computeNoticeCounts}.)
 *
 * ⚠ **Five honesties it is required to carry**, each of which the design would
 * otherwise leave for the user to discover:
 *
 * 1. **The counts are computed, never quoted.** See {@link computeNoticeCounts}.
 * 2. **The tier is a guess from the LAST provider, not a reading of the
 *    transcript.** A chat that ran on Versa and was later switched to a
 *    commercial model backfills *public* even though its transcript holds
 *    private-model work. There is no content scan and there will not be one, so
 *    the notice says this in one sentence and tells the user the one repair:
 *    switch it back to a private model and the next turn marks it.
 * 3. **The review list is broken down by provider**, because a `backfill:*` tier
 *    is the system's inference and not the user's assertion — unlike a `turn:*`
 *    or `mcp:*` tier, which records something that actually happened.
 * 4. **Knowledge bases start public whatever fed them** (AR-2), and the notice
 *    names the control that repairs it rather than only the exposure.
 * 5. **An enabled Public extension wired to clinical data stays reachable from a
 *    commercial model** (§13.5). This is the one item on the list that no
 *    refusal will ever teach: the other four announce something the user will
 *    run into, and this one announces something that will keep quietly working.
 *    It is also the only item that names specific things on this machine —
 *    `medcp`, on the operator's — so it is passed in rather than described.
 *
 * ⚠ **Dismissible, unlike `NonPrivateModelDisclosure`.** That one gates an
 * action and has a fact the user must be shown before taking it; this one
 * reports something that has already happened to data at rest. Trapping someone
 * in it buys nothing, and a modal that cannot be closed is the surest way to
 * make the next one go unread.
 */
export function FirstRunPrivacyNotice({
  open,
  onDismiss,
  counts,
  publicClinicalExtensions = [],
}: FirstRunPrivacyNoticeProps) {
  const [fetched, setFetched] = useState<NoticeCounts | null>(null);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    if (!open || counts) return;
    let cancelled = false;
    setFailed(false);
    fetchVisibleSessions()
      .then((sessions) => {
        if (!cancelled) setFetched(computeNoticeCounts(sessions));
      })
      .catch(() => {
        // Say so rather than rendering zeroes. A notice that silently reports
        // "0 conversations changed" because a fetch failed is worse than one
        // that admits it could not count: the user acts on the first.
        if (!cancelled) setFailed(true);
      });
    return () => {
      cancelled = true;
    };
  }, [open, counts]);

  const resolved = counts ?? fetched;

  return (
    <Dialog open={open} onOpenChange={(next) => !next && onDismiss()}>
      <DialogContent data-testid="first-run-privacy-notice" className="sm:max-w-[560px]">
        <DialogHeader>
          <DialogTitle>Some of your chats are now marked private</DialogTitle>
        </DialogHeader>

        <div className="space-y-4 text-sm text-text-default">
          {failed && (
            <DialogDescription
              data-testid="notice-count-error"
              className="text-sm text-text-default"
            >
              Biorouter could not read your chat list, so the numbers below are unavailable. Your
              chats are unchanged by this message.
            </DialogDescription>
          )}

          {!failed && !resolved && (
            <DialogDescription data-testid="notice-counting" className="text-sm text-text-default">
              Counting your chats…
            </DialogDescription>
          )}

          {!failed && resolved && (
            <>
              <DialogDescription
                data-testid="notice-headline"
                className="text-sm text-text-default"
              >
                <strong>{resolved.privateVisible}</strong> of your{' '}
                <strong>{resolved.totalVisible}</strong> chats are now marked private, because that
                is the model each of them was last using.
              </DialogDescription>

              {resolved.privateByProvider.length > 0 && (
                <div>
                  <p className="text-text-muted">Marked private by model:</p>
                  <ul data-testid="notice-by-provider" className="mt-1 space-y-0.5">
                    {resolved.privateByProvider.map(({ provider, count }) => (
                      <li key={provider} data-provider={provider}>
                        {providerLabel(provider)}: {count}
                      </li>
                    ))}
                  </ul>
                </div>
              )}

              {resolved.unknownProviderVisible > 0 && (
                <p data-testid="notice-unknown">
                  <strong>{resolved.unknownProviderVisible}</strong> chats record no model at all.
                  Biorouter cannot tell what those ran on, so they are left public.
                </p>
              )}
            </>
          )}

          <p data-testid="notice-last-model-caveat">
            Chats from before this version are marked by the model they were last using. If an older
            chat contains work you want kept private, switch it to a private model. It will be
            marked private from its next turn on.
          </p>

          <p data-testid="notice-knowledge-bases" className="text-text-muted">
            Knowledge bases that already existed start public, whatever fed them. If one holds
            private material you can mark it private yourself from the Knowledge view.
          </p>

          {/*
            §13.5's day-one extension disclosure. Rendered only when there is
            something to name — a paragraph that says "no extensions are
            affected" is noise on every machine that reads it.
          */}
          {publicClinicalExtensions.length > 0 && (
            <p data-testid="notice-public-clinical-extensions" className="text-text-muted">
              {publicClinicalExtensions.length === 1
                ? 'One extension you have enabled is set up for clinical data and is not marked private: '
                : 'Some extensions you have enabled are set up for clinical data and are not marked private: '}
              <strong>{publicClinicalExtensions.join(', ')}</strong>. Any model, including
              commercial models hosted outside your institution, can still call{' '}
              {publicClinicalExtensions.length === 1 ? 'it' : 'them'}. Nothing about{' '}
              {publicClinicalExtensions.length === 1 ? 'it' : 'them'} has changed. This notice is so
              you know.
            </p>
          )}
        </div>

        <div className="flex justify-end">
          <Button type="button" onClick={onDismiss} data-testid="notice-acknowledge">
            Got it
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
