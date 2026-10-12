import { useEffect } from 'react';
import { useConfig } from '../ConfigContext';
import { publicClinicalExtensions } from '../settings/extensions/extensionPrivacy';
import { FirstRunPrivacyNotice } from './FirstRunPrivacyNotice';
import {
  acknowledgeFirstRunNotice,
  FIRST_RUN_NOTICE_ACK_KEY,
  startFirstRunNoticeCount,
  useFirstRunNoticeDecision,
} from './firstRunNoticeDecision';
import { useFirstRunNoticeMayOpen } from './firstRunNoticeHandoff';

/**
 * Where the acknowledgement is recorded, and why it is not in the daemon, is written beside the key
 * in `firstRunNoticeDecision.ts`, which owns the count and the record (FR-R4-4). Re-exported so the
 * name has the same address it always had.
 */
export { FIRST_RUN_NOTICE_ACK_KEY };

/**
 * The install-wide mount of {@link FirstRunPrivacyNotice} (issue #56 §15.5(3)).
 *
 * ⚠ **This component is the task, as much as the notice is.** Task 38 ships a
 * one-way door: the migration marks a large share of an established user's
 * history private, and on the operator's machine that is 1,475 chats in one
 * step. §15.5 is titled "Day one must be shown, not discovered" and its third
 * clause says to run the backfill and show the counts *before enforcement
 * begins*. A notice component with no caller satisfies the first half of that
 * sentence and none of the second — the door still swings, and the sign
 * explaining it is in the source tree. Review found exactly that and called it
 * the blocker; this is the half that was missing.
 *
 * ⚠ **Mounted at the app level, outside the router.** Like
 * `AppNonPrivateModelDisclosureGate` next door, and for a weaker version of the
 * same reason: the subject is the install, not a view, and the user must not
 * have to navigate to History to be told History changed.
 *
 * ⚠ **Due only when the MIGRATION marked something visible.** Not "when a
 * private chat exists" — see `shouldShowFirstRunNotice` in `noticeCounts.ts`. A fresh install
 * never sees this, however many private chats it accumulates later.
 *
 * ⚠ **One fetch, and it settles the question for good.** The counts are read
 * once per launch, and only while unacknowledged; a machine that has dismissed
 * the notice does no work at all here. The extension list comes from the config
 * the app has already loaded.
 */
export function FirstRunPrivacyNoticeGate() {
  const { extensionsList } = useConfig();
  const decision = useFirstRunNoticeDecision();
  // Due is not the same as ready to open: on a slower launch the boot's count lands before the
  // splash has left and before the first page has drawn, and a dialog opened then sat over the
  // fading wordmark and an empty ground (FR-R5-1). Nothing waits on a surface the boot did not
  // reach, and the wait is bounded.
  const mayOpen = useFirstRunNoticeMayOpen(decision.status === 'show');

  // The boot starts the count as soon as the daemon is reachable (`renderer.tsx`), so by now it is
  // usually running or answered. This is for a surface the boot did not reach, and it is a no-op
  // when the count has already started (FR-R4-4).
  useEffect(() => {
    startFirstRunNoticeCount();
  }, []);

  if (decision.status !== 'show' || !mayOpen) return null;

  return (
    <FirstRunPrivacyNotice
      open
      counts={decision.counts}
      publicClinicalExtensions={publicClinicalExtensions(extensionsList ?? [])}
      onDismiss={acknowledgeFirstRunNotice}
    />
  );
}
