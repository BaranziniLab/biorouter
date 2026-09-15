import { render, act } from '@testing-library/react';
import { useRef } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { usePreviewMotion } from './usePreviewMotion';

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
});
