import { useEffect, useState } from 'react';
import { ModalShell } from '../ModalShell';
import { DialogDescription } from '../ui/dialog';
import { Button } from '../ui/button';
import { listSessions, type Session } from '../../api';
import { userActionHeaders } from '../../utils/userAction';
import {
  computeNoticeCounts,
  shouldShowFirstRunNotice,
  UNKNOWN_PROVIDER,
  type NoticeCounts,
} from './noticeCounts';

// The counting rules live in `noticeCounts.ts` (a leaf the boot can import without this dialog) and
// are re-exported under their old names, so every importer and test reads them from here as before.
export { computeNoticeCounts, shouldShowFirstRunNotice, UNKNOWN_PROVIDER };
export type { NoticeCounts };

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

/**
 * The id the count sentence carries, so the dialog's `aria-describedby` can name it. Whichever of
 * the three stand-ins renders (could not count, counting, the headline) wears it, and never two at
 * once, so the id is unique.
 */
const SUMMARY_ID = 'first-run-privacy-notice-summary';

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
 *
 * It is the shared dialog, not a shell of its own (R-34, first run round 1,
 * FR-R1-14): a `ModalShell` at size `md`, pinned at the top edge with the header
 * rule, the same anatomy as `AnnouncementModal`, the other "Got it" notice. On an
 * upgraded install it is the first thing on screen, so it is also the first thing
 * that shows whether the app wears one dialog or two. A presentation change
 * only: the copy, the props, the counting and every test id are as they were.
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
    <ModalShell
      open={open}
      onOpenChange={(next) => !next && onDismiss()}
      size="md"
      purpose="info"
      scrollBody
      title="Some of your chats are now marked private"
      // The count sentence (or its counting and could-not-count stand-ins: only one renders at a
      // time) is the dialog's description, exactly as it was when these were Radix descriptions.
      describedBy={SUMMARY_ID}
      footer={
        <Button type="button" onClick={onDismiss} data-testid="notice-acknowledge">
          Got it
        </Button>
      }
    >
      <div
        data-testid="first-run-privacy-notice"
        className="space-y-4 py-3 text-body text-text-default tabular-nums"
      >
        {failed && (
          <DialogDescription
            id={SUMMARY_ID}
            data-testid="notice-count-error"
            className="text-body text-text-default"
          >
            Biorouter could not read your chat list, so the numbers below are unavailable. Your
            chats are unchanged by this message.
          </DialogDescription>
        )}

        {!failed && !resolved && (
          <DialogDescription
            id={SUMMARY_ID}
            data-testid="notice-counting"
            className="text-body text-text-default"
          >
            Counting your chats…
          </DialogDescription>
        )}

        {!failed && resolved && (
          <>
            <DialogDescription
              id={SUMMARY_ID}
              data-testid="notice-headline"
              className="text-body text-text-default"
            >
              <strong className="font-semibold">{resolved.privateVisible}</strong> of your{' '}
              <strong className="font-semibold">{resolved.totalVisible}</strong> chats are now
              marked private, because that is the model each of them was last using.
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
                <strong className="font-semibold">{resolved.unknownProviderVisible}</strong> chats
                record no model at all. Biorouter cannot tell what those ran on, so they are left
                public.
              </p>
            )}
          </>
        )}

        <p data-testid="notice-last-model-caveat">
          Chats from before this version are marked by the model they were last using. If an older
          chat contains work you want kept private, switch it to a private model. It will be marked
          private from its next turn on.
        </p>

        <p data-testid="notice-knowledge-bases" className="text-text-muted">
          Knowledge bases that already existed start public, whatever fed them. If one holds private
          material you can mark it private yourself from the Knowledge view.
        </p>

        {/*
          §13.5's day-one extension disclosure. Rendered only when there is
          something to name — a paragraph that says "no extensions are
          affected" is noise on every machine that reads it.
        */}
        {publicClinicalExtensions.length > 0 && (
          <p data-testid="notice-public-clinical-extensions">
            {publicClinicalExtensions.length === 1
              ? 'One extension you have enabled is set up for clinical data and is not marked private: '
              : 'Some extensions you have enabled are set up for clinical data and are not marked private: '}
            <strong className="font-semibold">{publicClinicalExtensions.join(', ')}</strong>. Any
            model, including commercial models hosted outside your institution, can still call{' '}
            {publicClinicalExtensions.length === 1 ? 'it' : 'them'}. Nothing about{' '}
            {publicClinicalExtensions.length === 1 ? 'it' : 'them'} has changed. This notice is so
            you know.
          </p>
        )}
      </div>
    </ModalShell>
  );
}
