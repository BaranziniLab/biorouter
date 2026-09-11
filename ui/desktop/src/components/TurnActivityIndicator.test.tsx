import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';
import { act, render, screen } from '@testing-library/react';
import TurnActivityIndicator from './TurnActivityIndicator';

describe('TurnActivityIndicator', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => {
    vi.useRealTimers();
    vi.restoreAllMocks();
  });

  const renderAt = (sinceOffsetMs: number) => {
    const now = Date.now();
    vi.setSystemTime(now);
    return render(
      <TurnActivityIndicator
        activity={{ phase: 'thinking', label: 'Working on the result', since: now - sinceOffsetMs }}
      />
    );
  };

  it('shows the label immediately but withholds the number below the reveal threshold', () => {
    renderAt(0);
    // Twice on purpose: the visible label plus the sr-only announcement copy.
    expect(screen.getAllByText('Working on the result')).toHaveLength(2);
    expect(screen.queryByTestId('turn-activity-elapsed')).toBeNull();
  });

  it('reveals and advances the elapsed chip on the shared one-second tick', () => {
    renderAt(2000);
    act(() => {
      vi.advanceTimersByTime(1000);
    });
    expect(screen.getByTestId('turn-activity-elapsed')).toHaveTextContent('3s');
    act(() => {
      vi.advanceTimersByTime(1000);
    });
    expect(screen.getByTestId('turn-activity-elapsed')).toHaveTextContent('4s');
  });

  it('does not tick when the activity has no trustworthy origin', () => {
    // A reload mid-turn: render the pulse, but never a fabricated number.
    render(<TurnActivityIndicator activity={{ phase: 'running', label: 'Running 2 tools' }} />);
    act(() => {
      vi.advanceTimersByTime(5000);
    });
    expect(screen.getAllByText('Running 2 tools').length).toBeGreaterThan(0);
    expect(screen.queryByTestId('turn-activity-elapsed')).toBeNull();
  });

  it('surfaces the nudge past 45 seconds', () => {
    renderAt(46_000);
    expect(screen.getByText(/Still working/)).toBeInTheDocument();
  });

  it('keeps the nudge hidden before 45 seconds', () => {
    renderAt(10_000);
    expect(screen.queryByText(/Still working/)).toBeNull();
  });

  it('points at the composer only where there is one to stop from', () => {
    // A delegated subagent's tab in a browser has no composer (SD-8), so the
    // half of the nudge that sends the reader there is dropped, not left
    // pointing at nothing. The reassurance stays.
    const now = Date.now();
    vi.setSystemTime(now);
    render(
      <TurnActivityIndicator
        canStopHere={false}
        activity={{ phase: 'thinking', label: 'Working on the result', since: now - 46_000 }}
      />
    );
    expect(screen.getByText('Still working.')).toBeInTheDocument();
    expect(screen.queryByText(/stop the turn from the composer/)).toBeNull();
  });

  it('still names the composer by default', () => {
    renderAt(46_000);
    expect(
      screen.getByText('Still working. You can stop the turn from the composer.')
    ).toBeInTheDocument();
  });

  it('exposes a polite live region and hides the ticking chip from screen readers', () => {
    renderAt(5000);
    const status = screen.getByRole('status');
    expect(status).toHaveAttribute('aria-live', 'polite');
    expect(status).toHaveAttribute('aria-atomic', 'true');
    expect(screen.getByTestId('turn-activity-elapsed')).toHaveAttribute('aria-hidden', 'true');
  });

  it('announces elapsed time to screen readers in coarse buckets, not every second', () => {
    const { container } = renderAt(37_000);
    const srText = container.querySelector('.sr-only')?.textContent ?? '';
    // 37s falls in the 30s bucket — not "37 seconds", and not re-read per tick.
    expect(srText).toContain('about 30 seconds elapsed');
  });

  it('announces no duration at all below the first bucket', () => {
    const { container } = renderAt(5000);
    const srText = container.querySelector('.sr-only')?.textContent ?? '';
    expect(srText).toContain('Working on the result');
    expect(srText).not.toContain('seconds elapsed');
  });

  it('tags the phase on the container so styling and tests can key off it', () => {
    renderAt(0);
    expect(screen.getByTestId('turn-activity-indicator')).toHaveAttribute('data-phase', 'thinking');
  });

  it('shares ONE interval across every mounted indicator and tears it down on the last unmount', () => {
    const setInterval = vi.spyOn(window, 'setInterval');
    const clearInterval = vi.spyOn(window, 'clearInterval');
    const now = Date.now();
    vi.setSystemTime(now);

    const activity = { phase: 'thinking' as const, label: 'Thinking', since: now };
    const a = render(<TurnActivityIndicator activity={activity} />);
    const b = render(<TurnActivityIndicator activity={activity} />);
    const c = render(<TurnActivityIndicator activity={activity} />);

    expect(setInterval).toHaveBeenCalledTimes(1);

    a.unmount();
    b.unmount();
    expect(clearInterval).not.toHaveBeenCalled();

    c.unmount();
    expect(clearInterval).toHaveBeenCalledTimes(1);
  });

  /**
   * BR-61 — the visible half of the steer acknowledgement. The derive layer
   * decides WHEN to show it (utils/trailingActivity.test.ts); this is the
   * evidence that a user actually sees their own words when it does.
   */
  describe('steering', () => {
    const steerActivity = {
      phase: 'steering' as const,
      label: 'Steering the current turn',
      steerText: 'actually, use R',
      since: Date.now(),
    };

    it('shows the label and the user’s own words', () => {
      vi.setSystemTime(steerActivity.since);
      render(<TurnActivityIndicator activity={steerActivity} />);

      expect(screen.getByTestId('turn-activity-steer-text')).toHaveTextContent('actually, use R');
      // Visible label + the sr-only announcement.
      expect(screen.getAllByText(/Steering the current turn/)).not.toHaveLength(0);
    });

    it('tags the phase on the container so the surface is inspectable', () => {
      vi.setSystemTime(steerActivity.since);
      render(<TurnActivityIndicator activity={steerActivity} />);

      expect(screen.getByTestId('turn-activity-indicator')).toHaveAttribute(
        'data-phase',
        'steering'
      );
    });

    it('announces the steer text to screen readers, not just the generic label', () => {
      vi.setSystemTime(steerActivity.since);
      const { container } = render(<TurnActivityIndicator activity={steerActivity} />);

      const srOnly = container.querySelector('.sr-only');
      expect(srOnly?.textContent).toContain('actually, use R');
    });

    it('renders no steer chip for the ordinary phases', () => {
      render(
        <TurnActivityIndicator
          activity={{ phase: 'thinking', label: 'Thinking', since: Date.now() }}
        />
      );

      expect(screen.queryByTestId('turn-activity-steer-text')).toBeNull();
    });
  });
});
