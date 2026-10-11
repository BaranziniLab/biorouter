import { cleanup, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { Field } from '../ui/field';
import { NativeSelect } from './NativeSelect';

afterEach(cleanup);

/**
 * The parameter editors' native select, on the field skin (R-27). jsdom lays nothing out, so
 * what a render can prove is the contract: which classes the box carries (and which it must
 * not), that a `Field`'s id and aria props reach the `<select>` through the wrapper, and that
 * the chevron is decoration. `Layout/viewsFill.browser.test.tsx` measures the paint.
 */
describe('NativeSelect', () => {
  it('is the app’s field skin and names no ground, so the container’s ground decides', () => {
    render(
      <NativeSelect aria-label="Kind">
        <option value="a">A</option>
      </NativeSelect>
    );

    const select = screen.getByRole('combobox', { name: 'Kind' });
    expect(select).toHaveClass('br-field', 'h-control-md', 'rounded-element', 'appearance-none');
    // A `bg-*` utility would beat the layered field rule and leave an unfilled box (R-27).
    expect(select.className).not.toMatch(/\bbg-/);
    // The edge is the field's own: transparent at rest, drawn by `.br-field` when it means something.
    expect(select.className).not.toMatch(/border-border/);
  });

  it('draws the app’s chevron as decoration, in the wrapper and not over the pointer', () => {
    const { container } = render(
      <NativeSelect aria-label="Kind">
        <option value="a">A</option>
      </NativeSelect>
    );

    const chevron = container.querySelector('svg')!;
    expect(chevron).toHaveAttribute('aria-hidden', 'true');
    expect(chevron).toHaveClass('pointer-events-none', 'size-4', 'text-text-muted');
    expect(container.firstElementChild).toHaveClass('relative');
    expect(container.firstElementChild).toContainElement(screen.getByRole('combobox'));
  });

  it('receives a Field’s id and aria props on the select itself, so its label names it', () => {
    render(
      <Field id="kind" label="Kind of QC" error="Pick one">
        <NativeSelect>
          <option value="a">A</option>
        </NativeSelect>
      </Field>
    );

    const select = screen.getByLabelText('Kind of QC');
    expect(select.tagName).toBe('SELECT');
    expect(select).toHaveAttribute('id', 'kind');
    expect(select).toHaveAttribute('aria-invalid', 'true');
    expect(select).toHaveAccessibleDescription('Pick one');
  });

  it('is still the platform’s select: a choice changes its value and reaches onChange', async () => {
    const onChange = vi.fn();
    render(
      <NativeSelect aria-label="Kind" defaultValue="a" onChange={onChange}>
        <option value="a">A</option>
        <option value="b">B</option>
      </NativeSelect>
    );

    await userEvent.selectOptions(screen.getByRole('combobox'), 'b');
    expect(onChange).toHaveBeenCalledTimes(1);
    expect(screen.getByRole('combobox')).toHaveValue('b');
  });

  it('forwards its ref to the select', () => {
    const ref = { current: null as HTMLSelectElement | null };
    render(
      <NativeSelect ref={ref} aria-label="Kind">
        <option value="a">A</option>
      </NativeSelect>
    );

    expect(ref.current).toBe(screen.getByRole('combobox'));
  });
});
