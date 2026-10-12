import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { Checkbox } from './Checkbox';

const CSS = readFileSync(resolve(__dirname, '../../styles/main.css'), 'utf8');
/** The body of the first top-level rule whose selector is exactly `selector`. */
function rule(selector: string): string {
  const at = CSS.indexOf(`\n${selector} {`);
  expect(at, `no rule for ${selector}`).toBeGreaterThanOrEqual(0);
  const open = CSS.indexOf('{', at);
  return CSS.slice(open + 1, CSS.indexOf('}', open));
}

// jsdom has no layout engine and never runs Tailwind, so it cannot see the
// thing this component exists to fix — a bare `<input type="checkbox">` painting
// as macOS system blue in light mode and a bare white square in dark. What it
// can pin is the contract that lets the app do the painting at all, and the one
// piece of imperative state the component has to write by hand.
describe('Checkbox', () => {
  it('keeps a real, labellable input and hands the painting to the app', () => {
    render(
      <label>
        <Checkbox defaultChecked />
        Show subagent runs
      </label>
    );
    const input = screen.getByLabelText(/show subagent runs/i);
    expect(input).toHaveAttribute('type', 'checkbox');
    // Not `hidden`, not a div with a role: the native control stays in the tree
    // and in the tab order, so keyboard, form participation and the label
    // association all keep working. `appearance-none` + `opacity-0` is what
    // stops the OS drawing a second, un-themeable box on top of ours.
    expect(input).toHaveClass('appearance-none', 'opacity-0');
    expect((input as HTMLInputElement).checked).toBe(true);
  });

  /**
   * The measured defect (2026-09-11): the input was `peer sr-only`, a 1px
   * clipped box in the corner, so the 22px square everyone can see was a
   * picture. `ResetPanel`'s category boxes toggled on nothing at all and
   * `ExportAppDialog`'s toggled only on their text — the label sat *beside* the
   * box, not around it — while `SessionListView`'s worked only because a
   * `<label>` wraps it. A primitive whose correctness depends on every call site
   * remembering a wrapper is the bug; this is the contract that ends it.
   *
   * ⚠ Asserted at the source. jsdom computes no layout and routes a click to the
   * element the test names, so it cannot answer "does a click at the centre of
   * the square reach the input?" whatever the CSS says. That was measured in
   * Chromium instead (unchecked before, checked after); what is pinned here is
   * the geometry the browser needs in order to route it.
   */
  it('makes the input itself the hit target, so no call site has to wrap it', () => {
    render(<Checkbox aria-label="Include the vault" />);
    const input = screen.getByLabelText('Include the vault');

    // Stretched over the whole 24px target, on top of the paint...
    expect(input).toHaveClass('absolute', 'inset-0', 'h-full', 'w-full');
    // ...and not tucked away where only a keyboard or a label can reach it.
    expect(input).not.toHaveClass('sr-only');

    // Every painted sibling has to stay out of the way, or the topmost of them
    // swallows the click the input is there to receive.
    const painted = Array.from(input.parentElement?.children ?? []).filter(
      (child) => child !== input
    );
    expect(painted.length).toBeGreaterThan(0);
    for (const child of painted) expect(child).toHaveClass('pointer-events-none');
  });

  /**
   * Keyboard focus was invisible whichever way the input was hidden: the global
   * `input[type='checkbox']:focus-visible` rule in `main.css` answers focus with
   * `outline: none` and a background colour, and the input is transparent. The
   * indication has to live on the square that is actually painted.
   */
  it('puts the focus ring on the square the user can see', () => {
    render(<Checkbox aria-label="Include the vault" />);
    const box = screen.getByLabelText('Include the vault').nextElementSibling;
    expect(box).toHaveClass('br-checkbox-box');
    // Authored, not a utility at the call site (a new Tailwind string can fail to generate).
    expect(rule('.br-checkbox-input:focus-visible ~ .br-checkbox-box')).toMatch(
      /outline:\s*2px solid var\(--ring\)/
    );
  });

  /**
   * Spec 2.6: a 16px box with a 1.5px `--border-control` edge and radius 4 inside the 24px
   * target; checked, the accent fill with a 10px check; indeterminate, an 8×2 bar; the fill
   * over `--dur-fast-min`. jsdom runs no CSS, so the recipe is pinned at the source and measured
   * in `controlsGeometry.browser.test.tsx`.
   */
  it('draws a 16px box with the control edge, an accent fill and a trailing 10px check', () => {
    expect(rule('.br-checkbox')).toMatch(/inline-size:\s*24px/);
    const box = rule('.br-checkbox-box');
    expect(box).toMatch(/inset:\s*4px/);
    expect(box).toMatch(/border:\s*1\.5px solid var\(--border-control\)/);
    expect(box).toMatch(/border-radius:\s*var\(--radius-inner\)/);
    expect(box).toMatch(/background-color var\(--dur-fast-min\) var\(--ease-out\)/);
    expect(box).not.toMatch(/border-emphasized/);
    expect(
      rule(
        '.br-checkbox-input:checked ~ .br-checkbox-box,\n.br-checkbox-input:indeterminate ~ .br-checkbox-box'
      )
    ).toMatch(/background-color:\s*var\(--background-accent\)/);
    expect(rule('.br-checkbox-check')).toMatch(/inline-size:\s*10px/);
    expect(rule('.br-checkbox-bar')).toMatch(/inline-size:\s*8px;\s*block-size:\s*2px/);
  });

  it('shows the check when checked and the bar when indeterminate', () => {
    const { container, rerender } = render(<Checkbox aria-label="Pick" defaultChecked />);
    const svg = container.querySelector('svg.br-checkbox-check');
    expect(svg).toHaveAttribute('stroke-width', '2');
    expect(svg).toHaveAttribute('viewBox', '0 0 10 10');
    expect(container.querySelector('.br-checkbox-input:checked ~ .br-checkbox-check')).toBe(svg);
    rerender(<Checkbox aria-label="Pick" indeterminate />);
    expect(
      container.querySelector('.br-checkbox-input:indeterminate ~ .br-checkbox-bar')
    ).not.toBeNull();
  });

  it('writes the indeterminate state, which has no HTML attribute', () => {
    // React cannot set this declaratively — it is a DOM property only, so a
    // missing effect would leave a tri-state control silently stuck on
    // unchecked with no type error and no failing render.
    const { rerender } = render(<Checkbox aria-label="Select all" indeterminate />);
    const input = screen.getByLabelText('Select all') as HTMLInputElement;
    expect(input.indeterminate).toBe(true);

    rerender(<Checkbox aria-label="Select all" indeterminate={false} />);
    expect(input.indeterminate).toBe(false);
  });

  it('forwards a ref to the input, not to the wrapper', () => {
    let node: HTMLInputElement | null = null;
    render(
      <Checkbox
        aria-label="Ref target"
        ref={(el) => {
          node = el;
        }}
      />
    );
    expect((node as HTMLInputElement | null)?.tagName).toBe('INPUT');
  });
});
