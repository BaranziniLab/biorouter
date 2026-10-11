import { useState } from 'react';
import { act, fireEvent, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { Slider, type SliderStep } from './slider';

type Effort = 'quick' | 'normal' | 'deep';
const STEPS: SliderStep<Effort>[] = [
  { value: 'quick', label: 'Quick' },
  { value: 'normal', label: 'Normal' },
  { value: 'deep', label: 'Deep' },
];

const MAIN_CSS = readFileSync(join(__dirname, '../../styles/main.css'), 'utf8');

function Controlled({
  initial = 'normal',
  onChange,
  dir,
  disabled,
  description,
  describedBy,
  steps = STEPS,
  pageStep,
}: {
  initial?: string;
  onChange?: (value: string) => void;
  dir?: 'ltr' | 'rtl';
  disabled?: boolean;
  description?: string;
  describedBy?: string;
  steps?: SliderStep[];
  pageStep?: number;
}) {
  const [value, setValue] = useState<string>(initial);
  return (
    <div>
      <button type="button">Before</button>
      <Slider
        aria-label="Effort"
        steps={steps}
        value={value}
        dir={dir}
        disabled={disabled}
        description={description}
        aria-describedby={describedBy}
        pageStep={pageStep}
        data-testid="slider"
        onValueChange={(next) => {
          setValue(next);
          onChange?.(next);
        }}
      />
      <button type="button">After</button>
    </div>
  );
}

const thumb = () => screen.getByRole('slider', { name: 'Effort' });
const root = () => screen.getByTestId('slider');
const rail = () => root().querySelector('[data-slot="slider-rail"]') as HTMLElement;

/** Gives the rail a 200px box at x = 100 (jsdom lays nothing out). */
function placeRail(left = 100, width = 200) {
  vi.spyOn(rail(), 'getBoundingClientRect').mockReturnValue({
    left,
    right: left + width,
    width,
    top: 0,
    bottom: 24,
    height: 24,
    x: left,
    y: 0,
    toJSON: () => ({}),
  } as DOMRect);
}

afterEach(() => {
  vi.restoreAllMocks();
});

describe('Slider', () => {
  it('is a slider whose thumb speaks the stop label, with min, max and now as indexes', () => {
    render(<Controlled />);
    const slider = thumb();
    expect(slider).toHaveClass('br-slider-thumb');
    expect(slider).toHaveAttribute('aria-valuemin', '0');
    expect(slider).toHaveAttribute('aria-valuemax', '2');
    expect(slider).toHaveAttribute('aria-valuenow', '1');
    expect(slider).toHaveAttribute('aria-valuetext', 'Normal');
    expect(slider).toHaveAttribute('aria-orientation', 'horizontal');
    expect(slider).not.toHaveAttribute('aria-disabled');
    expect(root()).toHaveClass('br-slider');
    expect(root().style.getPropertyValue('--br-slider-fraction')).toBe('0.5');
  });

  it('is one Tab stop', async () => {
    const user = userEvent.setup();
    render(<Controlled />);
    act(() => screen.getByRole('button', { name: 'Before' }).focus());
    await user.tab();
    expect(thumb()).toHaveFocus();
    await user.tab();
    expect(screen.getByRole('button', { name: 'After' })).toHaveFocus();
  });

  it('draws a mark at every stop, on the track and on the fill', () => {
    render(<Controlled />);
    const track = [...root().querySelectorAll('[data-slot="slider-mark"]')] as HTMLElement[];
    const fill = [...root().querySelectorAll('[data-slot="slider-fill-mark"]')] as HTMLElement[];
    expect(track.map((mark) => mark.style.insetInlineStart)).toEqual(['0%', '50%', '100%']);
    expect(fill.map((mark) => mark.style.insetInlineStart)).toEqual(['0%', '50%', '100%']);
    expect(track.map((mark) => mark.dataset.value)).toEqual(['quick', 'normal', 'deep']);
    // Marks and fill are decoration: the thumb carries the value.
    expect(root().querySelector('[data-slot="slider-track"]')).toHaveAttribute(
      'aria-hidden',
      'true'
    );
    expect(root().querySelector('[data-slot="slider-fill"]')).toHaveAttribute(
      'aria-hidden',
      'true'
    );
  });

  it('raises with Right and Up, lowers with Left and Down, and jumps with Home and End', async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    render(<Controlled initial="quick" onChange={onChange} />);
    act(() => thumb().focus());

    await user.keyboard('{ArrowRight}');
    expect(thumb()).toHaveAttribute('aria-valuetext', 'Normal');
    await user.keyboard('{ArrowUp}');
    expect(thumb()).toHaveAttribute('aria-valuetext', 'Deep');
    await user.keyboard('{ArrowLeft}');
    expect(thumb()).toHaveAttribute('aria-valuetext', 'Normal');
    await user.keyboard('{ArrowDown}');
    expect(thumb()).toHaveAttribute('aria-valuetext', 'Quick');
    await user.keyboard('{End}');
    expect(thumb()).toHaveAttribute('aria-valuenow', '2');
    await user.keyboard('{Home}');
    expect(thumb()).toHaveAttribute('aria-valuenow', '0');
    expect(onChange.mock.calls.map(([value]) => value)).toEqual([
      'normal',
      'deep',
      'normal',
      'quick',
      'deep',
      'quick',
    ]);
  });

  it('moves by pageStep with Page Up and Page Down', async () => {
    const user = userEvent.setup();
    const steps = Array.from({ length: 11 }, (_, i) => ({ value: `s${i}`, label: `Step ${i}` }));
    render(<Controlled steps={steps} initial="s5" pageStep={3} />);
    act(() => thumb().focus());
    await user.keyboard('{PageUp}');
    expect(thumb()).toHaveAttribute('aria-valuetext', 'Step 8');
    await user.keyboard('{PageUp}');
    expect(thumb()).toHaveAttribute('aria-valuetext', 'Step 10');
    await user.keyboard('{PageDown}');
    expect(thumb()).toHaveAttribute('aria-valuetext', 'Step 7');
  });

  it('moves one stop per page on a three-stop scale by default', async () => {
    const user = userEvent.setup();
    render(<Controlled initial="quick" />);
    act(() => thumb().focus());
    await user.keyboard('{PageUp}');
    expect(thumb()).toHaveAttribute('aria-valuetext', 'Normal');
    await user.keyboard('{PageDown}');
    expect(thumb()).toHaveAttribute('aria-valuetext', 'Quick');
  });

  it('stays at an end without asking again, and still swallows the key', () => {
    const onChange = vi.fn();
    const outer = vi.fn();
    render(
      <div onKeyDown={outer}>
        <Controlled initial="deep" onChange={onChange} />
      </div>
    );
    const event = fireEvent.keyDown(thumb(), { key: 'ArrowRight' });
    // fireEvent returns false when the default was prevented.
    expect(event).toBe(false);
    expect(onChange).not.toHaveBeenCalled();
    expect(outer).not.toHaveBeenCalled();
    expect(thumb()).toHaveAttribute('aria-valuetext', 'Deep');
  });

  it('keeps its keys from a surrounding menu or list, and lets other keys through', () => {
    const outer = vi.fn();
    render(
      <div onKeyDown={(event) => outer(event.key)}>
        <Controlled initial="quick" />
      </div>
    );
    fireEvent.keyDown(thumb(), { key: 'ArrowRight' });
    fireEvent.keyDown(thumb(), { key: 'End' });
    fireEvent.keyDown(thumb(), { key: 'a' });
    fireEvent.keyDown(thumb(), { key: 'Escape' });
    expect(outer.mock.calls.map(([key]) => key)).toEqual(['a', 'Escape']);
  });

  it('passes a key with Alt, Ctrl or Meta through untouched', () => {
    const onChange = vi.fn();
    render(<Controlled initial="quick" onChange={onChange} />);
    for (const modifier of ['altKey', 'ctrlKey', 'metaKey'] as const) {
      expect(fireEvent.keyDown(thumb(), { key: 'ArrowRight', [modifier]: true })).toBe(true);
    }
    expect(onChange).not.toHaveBeenCalled();
  });

  it('runs from the right in a right-to-left layout: Left raises, Right lowers', async () => {
    const user = userEvent.setup();
    render(<Controlled initial="normal" dir="rtl" />);
    expect(root()).toHaveAttribute('dir', 'rtl');
    act(() => thumb().focus());
    await user.keyboard('{ArrowLeft}');
    expect(thumb()).toHaveAttribute('aria-valuetext', 'Deep');
    await user.keyboard('{ArrowRight}{ArrowRight}');
    expect(thumb()).toHaveAttribute('aria-valuetext', 'Quick');
    // Up and Down do not depend on the direction.
    await user.keyboard('{ArrowUp}');
    expect(thumb()).toHaveAttribute('aria-valuetext', 'Normal');
  });

  it('describes the thumb with the description, then the caller’s own ids', () => {
    render(
      <>
        <span id="effort-help">Applies to this chat.</span>
        <Controlled
          description="Quick answers fast, Deep thinks longer."
          describedBy="effort-help"
        />
      </>
    );
    const ids = thumb().getAttribute('aria-describedby')!.split(' ');
    expect(ids).toHaveLength(2);
    expect(ids[1]).toBe('effort-help');
    expect(document.getElementById(ids[0])).toHaveClass('sr-only');
    expect(thumb()).toHaveAccessibleDescription(
      'Quick answers fast, Deep thinks longer. Applies to this chat.'
    );
  });

  it('has no description node when none is given', () => {
    render(<Controlled />);
    expect(thumb()).not.toHaveAttribute('aria-describedby');
    expect(root().querySelector('[data-slot="slider-description"]')).toBeNull();
  });

  it('sets the nearest stop on a press, and snaps from stop to stop while dragged', () => {
    const onChange = vi.fn();
    render(<Controlled initial="quick" onChange={onChange} />);
    placeRail(100, 200);

    // 260 is 80% along: nearest stop is Deep (100%).
    fireEvent.pointerDown(root(), { clientX: 260, pointerId: 1, pointerType: 'mouse', button: 0 });
    expect(thumb()).toHaveAttribute('aria-valuetext', 'Deep');
    expect(root()).toHaveAttribute('data-dragging');

    // 190 is 45% along: Normal. Moving within the same stop asks nothing more.
    fireEvent.pointerMove(root(), { clientX: 190, pointerId: 1, pointerType: 'mouse' });
    fireEvent.pointerMove(root(), { clientX: 205, pointerId: 1, pointerType: 'mouse' });
    expect(thumb()).toHaveAttribute('aria-valuetext', 'Normal');

    // Past the start clamps to the first stop.
    fireEvent.pointerMove(root(), { clientX: -40, pointerId: 1, pointerType: 'mouse' });
    expect(thumb()).toHaveAttribute('aria-valuetext', 'Quick');

    fireEvent.pointerUp(root(), { clientX: -40, pointerId: 1, pointerType: 'mouse' });
    expect(root()).not.toHaveAttribute('data-dragging');
    // A move after release does nothing.
    fireEvent.pointerMove(root(), { clientX: 300, pointerId: 1, pointerType: 'mouse' });
    expect(thumb()).toHaveAttribute('aria-valuetext', 'Quick');

    expect(onChange.mock.calls.map(([value]) => value)).toEqual(['deep', 'normal', 'quick']);
  });

  it('measures from the right in a right-to-left layout', () => {
    render(<Controlled initial="normal" dir="rtl" />);
    placeRail(100, 200);
    // 120 is near the LEFT edge, which is the high end from the right.
    fireEvent.pointerDown(root(), { clientX: 120, pointerId: 1, pointerType: 'mouse', button: 0 });
    expect(thumb()).toHaveAttribute('aria-valuetext', 'Deep');
  });

  it('focuses the thumb on a press without its keyboard edge, until a key', () => {
    render(<Controlled initial="quick" />);
    placeRail();
    fireEvent.pointerDown(root(), { clientX: 200, pointerId: 1, pointerType: 'mouse', button: 0 });
    fireEvent.pointerUp(root(), { clientX: 200, pointerId: 1, pointerType: 'mouse' });
    expect(thumb()).toHaveFocus();
    expect(thumb()).toHaveAttribute('data-pointer-focus');
    fireEvent.keyDown(thumb(), { key: 'ArrowRight' });
    expect(thumb()).not.toHaveAttribute('data-pointer-focus');
  });

  it('drops the press marker on blur', () => {
    render(<Controlled />);
    placeRail();
    fireEvent.pointerDown(root(), { clientX: 200, pointerId: 1, pointerType: 'mouse', button: 0 });
    fireEvent.pointerUp(root(), { clientX: 200, pointerId: 1, pointerType: 'mouse' });
    act(() => screen.getByRole('button', { name: 'After' }).focus());
    expect(thumb()).not.toHaveAttribute('data-pointer-focus');
  });

  it('ignores a secondary or Ctrl press', () => {
    const onChange = vi.fn();
    render(<Controlled initial="quick" onChange={onChange} />);
    placeRail();
    fireEvent.pointerDown(root(), { clientX: 290, pointerId: 1, pointerType: 'mouse', button: 2 });
    fireEvent.pointerDown(root(), {
      clientX: 290,
      pointerId: 2,
      pointerType: 'mouse',
      button: 0,
      ctrlKey: true,
    });
    expect(onChange).not.toHaveBeenCalled();
    expect(root()).not.toHaveAttribute('data-dragging');
  });

  it('when disabled, leaves the Tab order and answers neither keys nor pointer', () => {
    const onChange = vi.fn();
    render(<Controlled initial="quick" disabled onChange={onChange} />);
    placeRail();
    expect(thumb()).not.toHaveAttribute('tabindex');
    expect(thumb()).toHaveAttribute('aria-disabled', 'true');
    expect(root()).toHaveAttribute('data-disabled');
    fireEvent.keyDown(thumb(), { key: 'End' });
    fireEvent.pointerDown(root(), { clientX: 290, pointerId: 1, pointerType: 'mouse', button: 0 });
    expect(onChange).not.toHaveBeenCalled();
  });

  it('shows the first stop for a value it does not know', () => {
    render(<Controlled initial="extreme" />);
    expect(thumb()).toHaveAttribute('aria-valuenow', '0');
    expect(thumb()).toHaveAttribute('aria-valuetext', 'Quick');
  });

  it('forwards its ref to the thumb', () => {
    let node: HTMLSpanElement | null = null;
    render(
      <Slider
        ref={(el) => {
          node = el;
        }}
        aria-label="Effort"
        steps={STEPS}
        value="normal"
        onValueChange={() => {}}
      />
    );
    expect(node).toBe(thumb());
  });

  describe('motion', () => {
    const slot = () => root().querySelector('[data-slot="slider-thumb-slot"]') as HTMLElement;
    const fill = () => root().querySelector('[data-slot="slider-fill"]') as HTMLElement;

    function External({ value }: { value: Effort }) {
      return (
        <Slider
          aria-label="Effort"
          data-testid="slider"
          steps={STEPS}
          value={value}
          onValueChange={() => {}}
        />
      );
    }

    it('mounts at rest with no still flag (CSS starts no transition on insertion)', () => {
      render(<External value="deep" />);
      expect(slot()).not.toHaveAttribute('data-motion-still');
      expect(slot()).toHaveAttribute('data-motion-layout');
      expect(fill()).toHaveAttribute('data-motion-layout');
    });

    it('snaps a value that changed without the person, in the same commit, then releases it', async () => {
      const raf = vi.spyOn(window, 'requestAnimationFrame');
      const { rerender } = render(<External value="quick" />);
      rerender(<External value="deep" />);
      // The new position and the flag arrive together.
      expect(root().style.getPropertyValue('--br-slider-fraction')).toBe('1');
      expect(slot()).toHaveAttribute('data-motion-still');
      expect(fill()).toHaveAttribute('data-motion-still');
      expect(raf).toHaveBeenCalled();
      await act(
        () =>
          new Promise<void>((done) =>
            window.requestAnimationFrame(() => window.requestAnimationFrame(() => done()))
          )
      );
      await act(async () => {});
      expect(slot()).not.toHaveAttribute('data-motion-still');
    });

    it('glides a change the person made', async () => {
      const user = userEvent.setup();
      render(<Controlled initial="quick" />);
      act(() => thumb().focus());
      await user.keyboard('{ArrowRight}');
      expect(slot()).not.toHaveAttribute('data-motion-still');
      expect(fill()).not.toHaveAttribute('data-motion-still');
    });
  });

  describe('authored CSS', () => {
    /** The declarations of the first rule whose selector list is exactly `selector`. */
    function rule(selector: string): string {
      const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
      const match = MAIN_CSS.match(new RegExp(`(?:^|\\n)${escaped}\\s*\\{([^}]*)\\}`));
      if (!match) throw new Error(`main.css has no rule for ${selector}`);
      return match[1];
    }

    it('sits in the WS-PRIMITIVES primitives block', () => {
      const start = MAIN_CSS.indexOf('/* @ws WS-PRIMITIVES begin: primitives */');
      const end = MAIN_CSS.indexOf('/* @ws WS-PRIMITIVES end: primitives */');
      const at = MAIN_CSS.indexOf('\n.br-slider {');
      expect(start).toBeGreaterThan(-1);
      expect(at).toBeGreaterThan(start);
      expect(at).toBeLessThan(end);
    });

    it('is a 24px target with a 4px track, 6px marks and a 16px thumb', () => {
      expect(rule('.br-slider')).toMatch(/block-size:\s*24px/);
      expect(rule('.br-slider-track::before,\n.br-slider-fill::before')).toMatch(
        /block-size:\s*4px/
      );
      expect(rule('.br-slider-mark')).toMatch(/inline-size:\s*6px/);
      const thumbRule = rule('.br-slider-thumb');
      expect(thumbRule).toMatch(/inline-size:\s*16px/);
      expect(thumbRule).toMatch(/block-size:\s*16px/);
      expect(thumbRule).toMatch(/background-color:\s*var\(--background-accent\)/);
    });

    it('fills the value in the accent and rests the marks on the control edge token', () => {
      expect(rule('.br-slider-mark')).toMatch(/var\(--border-control\)/);
      expect(rule('.br-slider-fill::before,\n.br-slider-fill > .br-slider-mark')).toMatch(
        /var\(--background-accent\)/
      );
    });

    it('moves only transform and clip-path, on one curve (principle 9)', () => {
      const slotRule = rule('.br-slider-thumb-slot');
      // The fill's own rule (the first `.br-slider-fill {` closes the shared track/fill list).
      const fillMotion = [...MAIN_CSS.matchAll(/\n\.br-slider-fill \{([^}]*)\}/g)]
        .map((m) => m[1])
        .find((body) => /clip-path:/.test(body))!;
      expect(slotRule).toMatch(/transition:\s*transform var\(--dur-fast-max\) var\(--ease-out\);/);
      expect(fillMotion).toMatch(
        /transition:\s*clip-path var\(--dur-fast-max\) var\(--ease-out\);/
      );
      const sliderBlock = MAIN_CSS.slice(
        MAIN_CSS.indexOf('\n.br-slider {'),
        MAIN_CSS.indexOf('/* Switch (`ui/switch.tsx`)')
      );
      const transitions = [...sliderBlock.matchAll(/transition:\s*([^;]+);/g)].map((m) => m[1]);
      for (const value of transitions) {
        expect(value).toMatch(/^(none|transform [^,]+|clip-path [^,]+)$/);
      }
    });

    it('draws keyboard focus on the thumb, never after a press', () => {
      expect(rule('.br-slider-thumb:focus-visible:not([data-pointer-focus])')).toMatch(
        /var\(--border-focus\)/
      );
    });

    it('rests in reduced motion and stays visible in forced colours', () => {
      const reduced = [...MAIN_CSS.matchAll(/@media \(prefers-reduced-motion: reduce\) \{/g)].map(
        (m) => MAIN_CSS.slice(m.index!, m.index! + 1200)
      );
      expect(reduced.some((block) => /\.br-slider-thumb-slot/.test(block))).toBe(true);
      expect(reduced.some((block) => /\.br-slider-fill,/.test(block))).toBe(true);
      const forced = MAIN_CSS.slice(MAIN_CSS.indexOf('/* Slider: the track and its marks'));
      expect(forced).toMatch(/\.br-slider-thumb \{\s*box-shadow: 0 0 0 2px Canvas;/);
      expect(forced).toMatch(/background-color: Highlight;/);
    });

    it('needs no slider package: it is built on plain React', () => {
      const source = readFileSync(join(__dirname, 'slider.tsx'), 'utf8');
      expect(source).not.toMatch(/from '@radix-ui\/react-slider'/);
      const pkg = JSON.parse(readFileSync(join(__dirname, '../../../package.json'), 'utf8'));
      expect(pkg.dependencies?.['@radix-ui/react-slider']).toBeUndefined();
    });
  });
});
