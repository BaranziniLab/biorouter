import { useEffect, useRef, useState } from 'react';
import { Greeting } from '../common/Greeting';
import { ActivityWindow } from '../../api';
import { HomeUsageCard, HomeUsageCardLoading } from './HomeUsageCard';
import { getCachedHomeActivity, refreshHomeActivity } from '../../utils/homeInsightsCache';
import { useHomeUsageFit } from './useHomeUsageFit';

/**
 * Home's content above the composer (spec 3.9): the greeting, then one usage
 * card. No eyebrow, no recent chats (the sidebar owns those), no paragraphs.
 *
 * The greeting and the card are one group, centred in the space above the
 * composer and on its column (`.biorouter-home-*` in main.css). The card yields
 * first (`homeUsageFit.ts`): it shows only at its whole measure, and where the
 * window is too short for it to leave the gap above the composer it drops its
 * heat map, then leaves. As the window gets shorter still, container queries
 * compact the greeting and then hide it; below that, Home scrolls.
 */
export function SessionInsights() {
  const initialActivity = useRef(getCachedHomeActivity()).current;
  const [activity, setActivity] = useState<ActivityWindow | null>(initialActivity);
  const [activityFailed, setActivityFailed] = useState(false);

  useEffect(() => {
    // Stale-while-revalidate: a cached window renders at once, and the card
    // shows its own loading state otherwise. The greeting never waits.
    const loadActivity = async () => {
      try {
        const refreshedActivity = await refreshHomeActivity();
        setActivity(refreshedActivity);
        setActivityFailed(false);
      } catch (error) {
        // A missing or old backend (404), or any failure, must not leave a
        // permanent skeleton where the card should be: collapse it instead.
        console.error('Failed to load activity:', error);
        if (!initialActivity) setActivityFailed(true);
      }
    };

    loadActivity();
  }, [initialActivity]);

  const insightsRef = useRef<HTMLDivElement>(null);
  const cardKind = activity ? 'loaded' : activityFailed ? null : 'loading';
  const fit = useHomeUsageFit(insightsRef, cardKind);

  return (
    <div ref={insightsRef} className="biorouter-home-insights">
      <div className="biorouter-home-column">
        <div className="biorouter-home-hero">
          {/* Home is where a new window and a new tab both land, and each is an
              arrival, so the unroll plays every time and the sentence is a fresh
              draw from the stock set. */}
          <Greeting className="biorouter-home-greeting text-title text-text-default" />
        </div>

        {cardKind ? (
          // Folded away by clipping, never unmounted, so the card keeps its natural size for
          // the fit to measure. `inert` keeps a folded card out of reach as well as out of sight.
          <div
            className="biorouter-home-usage"
            data-fit={fit}
            data-testid="home-usage"
            inert={fit === 'hidden'}
          >
            {activity ? <HomeUsageCard window={activity} /> : <HomeUsageCardLoading />}
          </div>
        ) : null}
      </div>
    </div>
  );
}
