import { render, act } from '@testing-library/react';
import { useRef } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { EASE_OUT } from '../../styles/motion';
import { previewMotionKeyframes, usePreviewMotion } from './usePreviewMotion';

function Host({
  open = true,
  layout = 'side',
  ready = true,
}: {
  open?: boolean;
  layout?: 'side' | 'stack';
  ready?: boolean;
}) {
  const ref = useRef<HTMLDivElement>(null);
  usePreviewMotion(ref, { isOpen: open, layout, ready });
  return (
    <div ref={ref}>
      <iframe title="persistent preview" />
    </div>
  );
}

afterEach(() => vi.restoreAllMocks());

function animations() {
  const results: Array<{
    cancel: ReturnType<typeof vi.fn>;
    playState: string;
    onfinish: (() => void) | null;
  }> = [];
  const animate = vi.fn<HTMLElement['animate']>(() => {
    const animation = {
      cancel: vi.fn(),
      playState: 'running',
      onfinish: null as (() => void) | null,
    };
    results.push(animation);
    return animation as unknown as Animation;
  });
  vi.stubGlobal(
    'matchMedia',
    vi.fn(() => ({ matches: false, addEventListener: vi.fn(), removeEventListener: vi.fn() }))
  );
  const original = HTMLElement.prototype.animate;
  HTMLElement.prototype.animate = animate;
  return {
    results,
    animate,
    restore: () => {
      HTMLElement.prototype.animate = original;
      vi.unstubAllGlobals();
    },
  };
}

describe('preview content motion', () => {
  it('preserves the frame and entrance through resize, then animates orientation and close', () => {
    const motion = animations();
    try {
      const { rerender, container } = render(<Host />);
      const frame = container.querySelector('iframe');
      expect(motion.animate.mock.calls[0][1]).toMatchObject({ duration: 300 });
      rerender(<Host layout="stack" />);
      expect(motion.animate).toHaveBeenCalledTimes(1);
      act(() => motion.results[0].onfinish?.());
      rerender(<Host layout="side" />);
      expect(motion.animate.mock.calls[1][1]).toMatchObject({ duration: 250 });
      rerender(<Host open={false} />);
      expect(motion.animate.mock.calls[2][1]).toMatchObject({ duration: 125 });
      expect(container.querySelector('iframe')).toBe(frame);
    } finally {
      motion.restore();
    }
  });

  it('waits until the stacked content has its geometry before starting entrance', () => {
    const motion = animations();
    try {
      const { rerender } = render(<Host layout="stack" ready={false} />);
      expect(motion.animate).not.toHaveBeenCalled();
      rerender(<Host layout="stack" ready />);
      expect(motion.animate.mock.calls[0][1]).toMatchObject({ duration: 300 });
    } finally {
      motion.restore();
    }
  });

  it('omits motion when reduce-motion is enabled', () => {
    const motion = animations();
    vi.mocked(window.matchMedia).mockReturnValue({
      matches: true,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
    } as unknown as MediaQueryList);
    try {
      const { rerender } = render(<Host />);
      rerender(<Host layout="stack" />);
      rerender(<Host open={false} />);
      expect(motion.animate).not.toHaveBeenCalled();
    } finally {
      motion.restore();
    }
  });

  it('runs on the token curve, never a private one (spec 2.4)', () => {
    const motion = animations();
    try {
      const { rerender } = render(<Host />);
      act(() => motion.results[0].onfinish?.());
      rerender(<Host open={false} />);
      for (const call of motion.animate.mock.calls) {
        expect(call[1]).toMatchObject({ easing: EASE_OUT });
      }
    } finally {
      motion.restore();
    }
  });

  it('snaps a side and stack crossing while the window is being resized', () => {
    const motion = animations();
    try {
      const { rerender } = render(<Host />);
      act(() => motion.results[0].onfinish?.());
      document.body.classList.add('biorouter-window-resizing');
      rerender(<Host layout="stack" />);
      expect(motion.animate).toHaveBeenCalledTimes(1);
      document.body.classList.remove('biorouter-window-resizing');
      // An action crossing back plays the crossfade.
      rerender(<Host layout="side" />);
      expect(motion.animate).toHaveBeenCalledTimes(2);
      expect(motion.animate.mock.calls[1][1]).toMatchObject({ duration: 250 });
    } finally {
      document.body.classList.remove('biorouter-window-resizing');
      motion.restore();
    }
  });
});

describe('preview motion keyframes', () => {
  it('enters 32px from the right beside the chat and leaves half as far', () => {
    expect(previewMotionKeyframes('open', 'side')[0]).toEqual({
      opacity: 0,
      transform: 'translateX(32px)',
    });
    expect(previewMotionKeyframes('close', 'side')[1]).toEqual({
      opacity: 0,
      transform: 'translateX(16px)',
    });
  });

  it('drops a sheet 16px from under the header and lets it fade out in place', () => {
    expect(previewMotionKeyframes('open', 'stack')[0]).toEqual({
      opacity: 0,
      transform: 'translateY(-16px)',
    });
    expect(previewMotionKeyframes('close', 'stack')).toEqual([{ opacity: 1 }, { opacity: 0 }]);
  });

  it('crossfades a crossing with no travel', () => {
    expect(previewMotionKeyframes('orientation', 'stack')).toEqual([
      { opacity: 0 },
      { opacity: 1 },
    ]);
  });
});

describe('the panel motion marker', () => {
  function PanelHost({ open }: { open: boolean }) {
    const ref = useRef<HTMLDivElement>(null);
    usePreviewMotion(ref, { isOpen: open, layout: 'side', ready: true });
    return (
      <aside className="br-preview-panel" data-testid="panel">
        <div ref={ref} />
      </aside>
    );
  }

  // The strip and the bar fade on this marker (preview-panel.css), so it must be
  // on the panel for exactly as long as the body's own motion runs.
  it('marks the panel while the body opens and clears it when the motion ends', () => {
    const motion = animations();
    try {
      const { getByTestId, rerender } = render(<PanelHost open={false} />);
      expect(getByTestId('panel').dataset.previewMotion).toBeUndefined();
      rerender(<PanelHost open />);
      expect(getByTestId('panel').dataset.previewMotion).toBe('open');
      act(() => motion.results[0].onfinish?.());
      expect(getByTestId('panel').dataset.previewMotion).toBeUndefined();
      rerender(<PanelHost open={false} />);
      expect(getByTestId('panel').dataset.previewMotion).toBe('close');
    } finally {
      motion.restore();
    }
  });
});
