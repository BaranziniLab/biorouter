import { useState } from 'react';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';

import CustomRadio from './CustomRadio';

/**
 * Q2-11 (live QA round 2). The real `<input type="radio">` is `sr-only`, so the
 * focus ring was drawn on a 1×1px clipped box and the checked dot is a fill
 * that forced colours erase. main.css now draws focus on the visible RING and
 * redraws ring and dot under forced colours, keyed on `data-radio-ring` /
 * `data-radio-dot` and on the input being their PRECEDING sibling
 * (`input[type='radio']:focus-visible ~ [data-radio-ring]`).
 *
 * jsdom has no `:focus-visible`, no forced colours and no cascade, so what is
 * decidable here is the structure those rules need, and the checked wiring; the
 * rules themselves are pinned at the source in `styles/focusFallback.test.ts`.
 */
function Group({ initial = 'private' }: { initial?: string }) {
  const [value, setValue] = useState(initial);
  return (
    <div role="radiogroup" aria-label="Privacy">
      {['private', 'public'].map((option) => (
        <CustomRadio
          key={option}
          id={`privacy-${option}`}
          name="privacy"
          value={option}
          checked={value === option}
          onChange={(event) => setValue(event.target.value)}
          label={option === 'private' ? 'Private' : 'Public'}
        />
      ))}
    </div>
  );
}

const hooks = (radio: HTMLElement) => {
  const box = radio.parentElement!;
  return {
    box,
    ring: box.querySelector<HTMLElement>('[data-radio-ring]'),
    dot: box.querySelector<HTMLElement>('[data-radio-dot]'),
  };
};

describe('CustomRadio', () => {
  it('marks the visible ring and dot for the focus and forced-colours rules', () => {
    render(<Group />);
    const radio = screen.getByRole('radio', { name: 'Private' });
    const { box, ring, dot } = hooks(radio);

    expect(ring).not.toBeNull();
    expect(dot).not.toBeNull();
    // The rules use the general-sibling combinator from the input, so the input
    // must share the box with both and come before them.
    expect(ring!.parentElement).toBe(box);
    expect(dot!.parentElement).toBe(box);
    expect(box.firstElementChild).toBe(radio);
    expect(radio.compareDocumentPosition(ring!) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(radio.compareDocumentPosition(dot!) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    // Decorative: the input is the one thing a screen reader should meet.
    expect(ring).toHaveAttribute('aria-hidden', 'true');
    expect(dot).toHaveAttribute('aria-hidden', 'true');
  });

  it('keeps the real input as the checked, focusable control', () => {
    render(<Group />);
    const privateRadio = screen.getByRole('radio', { name: 'Private' });
    const publicRadio = screen.getByRole('radio', { name: 'Public' });

    expect(privateRadio).toBeChecked();
    expect(publicRadio).not.toBeChecked();
    expect(privateRadio).toHaveAttribute('type', 'radio');
    expect(privateRadio).toHaveAttribute('name', 'privacy');
    expect(privateRadio.className).toContain('peer');
  });

  it('moves the checked state with the input, which is what the rules read', async () => {
    const user = userEvent.setup();
    render(<Group />);
    const privateRadio = screen.getByRole('radio', { name: 'Private' });
    const publicRadio = screen.getByRole('radio', { name: 'Public' });

    await user.click(screen.getByText('Public'));
    expect(publicRadio).toBeChecked();
    expect(privateRadio).not.toBeChecked();
    // The checked dot is the one after the checked input: `:checked ~ [data-radio-dot]`.
    expect(hooks(publicRadio).box.querySelector('input:checked ~ [data-radio-dot]')).toBe(
      hooks(publicRadio).dot
    );
    expect(hooks(privateRadio).box.querySelector('input:checked ~ [data-radio-dot]')).toBeNull();
  });

  it('is reached by Tab on its input, so the ring rule has a focused input to follow', async () => {
    const user = userEvent.setup();
    render(<Group />);
    await user.tab();
    const focused = document.activeElement as HTMLElement;
    expect(focused).toBe(screen.getByRole('radio', { name: 'Private' }));
    expect(hooks(focused).box.querySelector('input:focus ~ [data-radio-ring]')).toBe(
      hooks(focused).ring
    );
  });

  it('dims but keeps the hooks when disabled, for the GrayText forced-colours rule', () => {
    render(
      <CustomRadio
        id="locked"
        name="locked"
        value="locked"
        checked={false}
        onChange={() => {}}
        disabled
        label="Locked"
      />
    );
    const radio = screen.getByRole('radio', { name: 'Locked' });
    expect(radio).toBeDisabled();
    expect(hooks(radio).box.querySelector('input:disabled ~ [data-radio-ring]')).not.toBeNull();
    expect(hooks(radio).box.querySelector('input:disabled ~ [data-radio-dot]')).not.toBeNull();
  });
});
