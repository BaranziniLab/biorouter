import { act, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { CrewScreen } from '../state/crewStatus';
import { makeController, renderWithController } from '../timeline/timelineTestUtils';
import { CREW_SKELETON_DELAY_MS } from './CrewSkeleton';
import { MainScreen } from './MainScreen';

/**
 * Q2-59: the main area stood blank white for about two seconds while Crew connected, with only
 * "Checking connection" in the small status line to say anything was happening. Every screen that
 * only waits (`loading`, `connecting`, `checking`) now shows something: the setup card, or the
 * message skeleton 150ms into the wait — counted once per wait, so a hand-off from one waiting
 * screen to the next never starts a fresh blank.
 */

afterEach(() => {
  vi.useRealTimers();
});

const body = () => document.querySelector<HTMLElement>('.crew-frame-screen-body') as HTMLElement;
const bones = () => document.querySelectorAll('.crew-frame-bone-message').length;

function renderScreen(screenName: CrewScreen) {
  const connection = {
    id: 'conn-1',
    name: 'Fixture',
    ssh_target: 'alice@hpc.example.edu',
    status: 'connected',
  } as never;
  const controller = makeController({
    screen: screenName,
    snapshot: null,
    channel: null,
    connection,
    connections: [connection],
  });
  const view = renderWithController(<MainScreen withBand />, controller);
  return {
    ...view,
    moveTo(next: CrewScreen) {
      view.rerenderWith({ ...controller, screen: next });
    },
  };
}

describe('the main area while Crew connects or checks (Q2-59)', () => {
  it.each<CrewScreen>(['loading', 'checking'])(
    '%s shows the message skeleton 150ms in, with its wait named at once',
    (screenName) => {
      vi.useFakeTimers();
      renderScreen(screenName);
      // Named for a screen reader at once; drawn once the wait is long enough to be seen.
      expect(screen.getByRole('status')).not.toBeEmptyDOMElement();
      act(() => {
        vi.advanceTimersByTime(CREW_SKELETON_DELAY_MS);
      });
      expect(bones()).toBe(4);
      expect(body().textContent?.trim()).not.toBe('');
    }
  );

  it('connecting shows its setup card at once', () => {
    renderScreen('connecting');
    expect(screen.getByTestId('crew-connecting')).toBeVisible();
    expect(body().textContent?.trim()).not.toBe('');
  });

  it('never blanks again when connecting hands over to checking', () => {
    vi.useFakeTimers();
    const view = renderScreen('connecting');
    act(() => {
      vi.advanceTimersByTime(2000);
    });
    // The connect lands: the skeleton that takes over draws at once, not 150ms later.
    view.moveTo('checking');
    expect(bones()).toBe(4);
  });

  it('starts the count over for a new wait', () => {
    vi.useFakeTimers();
    const view = renderScreen('checking');
    act(() => {
      vi.advanceTimersByTime(CREW_SKELETON_DELAY_MS);
    });
    expect(bones()).toBe(4);
    view.moveTo('offline');
    view.moveTo('checking');
    // A fresh wait: nothing flashes for a check that is over quickly.
    expect(bones()).toBe(0);
    act(() => {
      vi.advanceTimersByTime(CREW_SKELETON_DELAY_MS);
    });
    expect(bones()).toBe(4);
  });
});
