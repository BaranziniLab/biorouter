import { useEffect, useState } from 'react';
import type { SessionClassification } from '../../api';
import {
  getCachedSessionList,
  preloadSessionList,
  subscribeSessionList,
} from '../../utils/sessionListCache';
import { sessionTiersDiffer } from './sessionTier';

function readCachedTiers(): Record<string, SessionClassification> {
  const tiers: Record<string, SessionClassification> = {};
  for (const session of getCachedSessionList() ?? []) {
    if (session.privacy_tier) tiers[session.id] = session.privacy_tier;
  }
  return tiers;
}

/**
 * Privacy tier per session id, as the shared session-list cache reports it
 * (issue #56, R10).
 *
 * One reader for the surfaces that draw a chat's tier without holding its row:
 * the tab strips (`ChatGroupsShell`, which folds this with its live and
 * outside-the-list sources) and a schedule's run list (`ScheduleDetailView`,
 * whose endpoint carries no tier at all).
 *
 * An id the list does not carry is ABSENT from the result — not `public`. The
 * list is `GET /sessions?include_subagents=false`, and it INNER JOINs
 * `messages`, so a delegated subagent's chat and a chat that has recorded
 * nothing are legitimately missing; `ChatKindIcon` draws such an id as not yet
 * known. Asserting Public for a chat nobody has read is the lie
 * `privacy/sessionTier.ts` exists to rule out.
 *
 * # It warms the cache itself
 *
 * `preloadSessionList()` returns early when the cache is non-null and swallows
 * its own errors, so this costs one fetch on a cold start and nothing after.
 * Nothing else can be relied on to have filled it: the only incidental warmer
 * on a normal launch is the Hub index route, absent in a window that opens
 * straight onto a chat or a schedule.
 *
 * ⚠ Subscribe BEFORE asking for the fetch. `preloadSessionList` is async but
 * makes no promise about when it resolves, and a cache that landed between the
 * first read and the subscription would emit to nobody and leave every glyph
 * not-yet-known until the next unrelated change.
 *
 * ⚠ **Seeded from the cache on the FIRST render, not in the effect.** An effect
 * runs after paint, so an empty initial map drew every tab of a remounted shell
 * without a tier for one frame — Public until 2026-09-14, and a dimmed
 * not-yet-known glyph that fades up after it. The cache is module state that
 * survives a route change, so when it is warm (a Settings round-trip) the
 * answer is available before the first paint and there is no reason to paint
 * without it.
 */
export function useSessionListTiers(): Record<string, SessionClassification> {
  const [tiers, setTiers] = useState<Record<string, SessionClassification>>(readCachedTiers);

  useEffect(() => {
    const read = () => {
      const next = readCachedTiers();
      // Identity-stable when nothing changed, so a list refresh that touched
      // no tier re-renders nothing downstream.
      setTiers((prev) => (sessionTiersDiffer(prev, next) ? next : prev));
    };
    // Again here: the cache may have changed between the first render and this
    // effect, and nothing would announce it to a subscriber that did not exist.
    read();
    const unsubscribe = subscribeSessionList(read);
    preloadSessionList();
    return unsubscribe;
  }, []);

  return tiers;
}
