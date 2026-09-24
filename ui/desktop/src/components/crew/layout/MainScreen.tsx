import { useId, useState } from 'react';
import type { CrewScreen } from '../state/crewStatus';
import { Button } from '../../ui/button';
import { ConnectionBar } from '../channel';
import { OnboardingScreen } from '../onboarding';
import { workspaceTitle } from '../sidebar';
import { useCrew } from '../state/CrewControllerContext';
import { layoutCopy } from './copy';
import { MessagesSkeleton } from './CrewSkeleton';

/** What `updates-paused` offers when there is no saved connection for the bar to retry. */
function UpdatesPaused() {
  const crew = useCrew();
  return (
    <div className="crew-frame-paused" data-testid="crew-updates-paused">
      <p>{layoutCopy.updatesPaused}</p>
      {!crew.connection && (
        <Button
          type="button"
          variant="secondary"
          size="sm"
          disabled={crew.isPending('refresh')}
          onClick={() => void crew.act('global', 'refresh', () => crew.refresh())}
        >
          {layoutCopy.tryAgain}
        </Button>
      )}
    </div>
  );
}

/** The screens that only wait for Crew: the main area must never stand blank through them. */
const WAITING_SCREENS: readonly CrewScreen[] = ['loading', 'connecting', 'checking'];

/**
 * When the main area started waiting (`loading`, `connecting`, `checking`), across moves between
 * those screens, or null while it is not waiting. A connect hands `connecting` to `checking`, and
 * the skeleton that mounts then must not start its 150ms over: the wait already happened.
 */
function useWaitingSince(screen: CrewScreen): number | null {
  const waiting = WAITING_SCREENS.includes(screen);
  const [since, setSince] = useState<number | null>(() => (waiting ? Date.now() : null));
  if (waiting && since === null) setSince(Date.now());
  if (!waiting && since !== null) setSince(null);
  return waiting ? since : null;
}

/**
 * The main area outside a channel (ui-redesign-spec, "Main-area states outside a channel"): the
 * screen `deriveCrewScreen()` chose, and nothing else decides it.
 *
 * - `loading` and `checking` draw message-shaped placeholders 150ms into the wait, counted from
 *   when the area started waiting (a `connecting` card before it counts), and `connecting` shows
 *   its setup card: the area is never blank while Crew connects or checks (Q2-59);
 * - `updates-paused` is the connection bar (with its Retry) and one line saying why nothing shows;
 * - every other screen belongs to onboarding: first run, connecting, offline, sign in, trust,
 *   not set up, join, no team and no channel.
 *
 * The connection bar is mounted on every screen, because it is where an error whose own surface
 * is not on screen renders — exactly once. With the Crew sidebar beside it, the area opens with an
 * empty 44px band so the top edge stays one line; the page's `<h1>` names the workspace there.
 */
export function MainScreen({ withBand }: { withBand: boolean }) {
  const crew = useCrew();
  const headingId = useId();
  const { screen } = crew;
  const waitingSince = useWaitingSince(screen);
  const title =
    (crew.connection && workspaceTitle(crew.snapshot, crew.connections, crew.connectionId)) ||
    layoutCopy.crew;

  let content;
  switch (screen) {
    case 'loading':
      content = <MessagesSkeleton label={layoutCopy.loading} since={waitingSince ?? undefined} />;
      break;
    case 'checking':
      content = (
        <MessagesSkeleton label={layoutCopy.loadingChannels} since={waitingSince ?? undefined} />
      );
      break;
    case 'updates-paused':
      content = <UpdatesPaused />;
      break;
    default:
      content = <OnboardingScreen />;
  }

  const heading = (
    <h1 id={headingId} className="sr-only">
      {title}
    </h1>
  );
  return (
    <section className="crew-frame-screen" aria-labelledby={headingId} data-crew-screen={screen}>
      {withBand ? <div className="crew-frame-band">{heading}</div> : heading}
      <ConnectionBar />
      <div className="crew-frame-screen-body">{content}</div>
    </section>
  );
}
