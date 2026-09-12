import { useEffect, useState } from 'react';
import { useConfig } from '../ConfigContext';
import { publicClinicalExtensions } from '../settings/extensions/extensionPrivacy';
import {
  computeNoticeCounts,
  FirstRunPrivacyNotice,
  shouldShowFirstRunNotice,
  type NoticeCounts,
} from './FirstRunPrivacyNotice';
import { listSessions, type Session } from '../../api';
import { userActionHeaders } from '../../utils/userAction';

/**
 * Where "this machine has already been told" is recorded.
 *
 * ⚠ **localStorage, unlike the sibling non-private-model disclosure, which
 * records its acknowledgement in the daemon.** That one gates an action and its
 * acknowledgement is a fact about the user: DR-16 makes it carry a proof-of-user
 * header and the daemon refuses to record it without one. This one gates
 * nothing. It reports something that has already happened to data at rest, it is
 * dismissible, and the worst outcome of losing the record is that a user sees an
 * accurate one-screen summary twice. Paying for a route, a store, an OpenAPI
 * regeneration and a proof-of-user round trip to prevent that would be spending
 * the security machinery on a receipt.
 *
 * The cost is stated rather than hidden: clearing browser storage, or a second
 * machine sharing the same database, shows the notice again. Both are correct
 * behaviours for a notice whose subject is "your history changed".
 */
export const FIRST_RUN_NOTICE_ACK_KEY = 'biorouter-privacy-first-run-notice-ack';

function alreadyAcknowledged(): boolean {
  try {
    return localStorage.getItem(FIRST_RUN_NOTICE_ACK_KEY) === '1';
  } catch {
    // Storage disabled or full. Treat as "not acknowledged" — showing an
    // accurate notice one extra time is the harmless direction, and the write
    // below fails the same way, so this cannot loop silently on a launch that
    // would otherwise have been quiet.
    return false;
  }
}

function recordAcknowledged(): void {
  try {
    localStorage.setItem(FIRST_RUN_NOTICE_ACK_KEY, '1');
  } catch {
    // See above. Nothing the user can do about it and nothing to say.
  }
}

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
 * private chat exists" — see {@link shouldShowFirstRunNotice}. A fresh install
 * never sees this, however many private chats it accumulates later.
 *
 * ⚠ **One fetch, and it settles the question for good.** The counts are read
 * once per launch, and only while unacknowledged; a machine that has dismissed
 * the notice does no work at all here. The extension list comes from the config
 * the app has already loaded.
 */
export function FirstRunPrivacyNoticeGate() {
  const { extensionsList } = useConfig();
  const [counts, setCounts] = useState<NoticeCounts | null>(null);
  const [dismissed, setDismissed] = useState(() => alreadyAcknowledged());

  useEffect(() => {
    if (dismissed) return;
    let cancelled = false;
    // With the user's proof: without it the daemon omits every private chat,
    // and the notice would count none (issue #56, QA 2026-09-10 M1).
    userActionHeaders()
      .then((headers) =>
        listSessions<true>({ throwOnError: true, headers, query: { include_subagents: false } })
      )
      .then((response) => {
        if (!cancelled) setCounts(computeNoticeCounts(response.data.sessions as Session[]));
      })
      .catch(() => {
        // The daemon is not answering yet, or not at all. Say nothing and leave
        // the record unwritten, so the next launch asks again — the alternative
        // is burning the one-time notice on a launch where it could not have
        // been shown.
      });
    return () => {
      cancelled = true;
    };
  }, [dismissed]);

  if (dismissed || !counts || !shouldShowFirstRunNotice(counts)) return null;

  return (
    <FirstRunPrivacyNotice
      open
      counts={counts}
      publicClinicalExtensions={publicClinicalExtensions(extensionsList ?? [])}
      onDismiss={() => {
        recordAcknowledged();
        setDismissed(true);
      }}
    />
  );
}
