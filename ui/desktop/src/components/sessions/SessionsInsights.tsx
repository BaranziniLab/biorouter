import { useEffect, useRef, useState } from 'react';
import { Greeting } from '../common/Greeting';
import { ActivityWindow } from '../../api';
import { HomeUsageCard, HomeUsageCardLoading } from './HomeUsageCard';
import { getCachedHomeActivity, refreshHomeActivity } from '../../utils/homeInsightsCache';

/**
 * Home's content above the composer (spec 3.9): the greeting, then one usage
 * card. No eyebrow, no recent chats (the sidebar owns those), no paragraphs.
 *
 * The greeting and the card are one group, centred in the space above the
 * composer and on its column (`.biorouter-home-*` in main.css). As the window
 * gets short, container queries compact the hero and then hide it; below that,
 * Home scrolls.
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

  return (
    <div className="biorouter-home-insights">
      <div className="biorouter-home-column">
        <div className="biorouter-home-hero">
          {/* Home is where a new window and a new tab both land, and each is an
              arrival, so the unroll plays every time and the sentence is a fresh
              draw from the stock set. */}
          <Greeting className="biorouter-home-greeting text-title text-text-default" />
        </div>

        {activity ? (
          <HomeUsageCard window={activity} />
        ) : activityFailed ? null : ( // definitive failure: collapse, don't leave a void
          <HomeUsageCardLoading />
        )}
      </div>
    </div>
  );
}
