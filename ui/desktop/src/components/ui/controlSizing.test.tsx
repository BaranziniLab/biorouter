import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import { Button } from './button';
import { Input } from './input';
import { Textarea } from './textarea';

afterEach(cleanup);

describe('shared controls follow the font-size spacing preference', () => {
  it.each([
    ['xs', 'compact'],
    ['sm', 'sm'],
    ['default', 'md'],
    ['lg', 'lg'],
  ] as const)('keeps %s icon buttons square using one scalable size token', (size, token) => {
    render(<Button size={size} shape="round" aria-label="Action" />);
    expect(screen.getByRole('button', { name: 'Action' })).toHaveClass(
      `h-control-${token}`,
      `w-control-${token}`
    );
  });

  it('uses the same scalable default height for buttons and inputs', () => {
    render(
      <>
        <Button>Save</Button>
        <Input aria-label="Name" />
      </>
    );
    expect(screen.getByRole('button', { name: 'Save' })).toHaveClass('h-control-md');
    expect(screen.getByRole('textbox', { name: 'Name' })).toHaveClass('h-control-md');
  });

  it('still lets callers request an unboxed button', () => {
    render(<Button className="h-auto p-0">Inline action</Button>);
    expect(screen.getByRole('button')).toHaveClass('h-auto');
    expect(screen.getByRole('button')).not.toHaveClass('h-control-md');
  });
});

/**
 * R-27 (owner message 13, design.md 9.11): a text field is a subtle solid fill with no resting
 * outline. It rests on `--background-fill` with its 1px border kept TRANSPARENT (forced colours
 * draw it; nothing moves when an edge appears), steps to `--background-fill-strong` under the
 * pointer unless it is focused or disabled, and draws an edge only for focus (`--border-focus`)
 * and for an error (`--border-danger`, which wins). The look is authored (`.br-field`) and
 * layered, so a caller's utility still wins.
 */
describe('text fields are filled, with an edge only for focus and error', () => {
  const CSS = readFileSync(resolve(__dirname, '../../styles/main.css'), 'utf8');
  // The rules sit in `@layer components`, indented one level.
  const OPEN = '@layer components {\n  .br-field,\n  .br-fill-rest {';
  const layer = CSS.slice(CSS.indexOf(OPEN));
  const body = (selector: string) => {
    const at = layer.indexOf(`\n  ${selector.replace(/\n/g, '\n  ')} {`);
    expect(at, `no layered rule for ${selector}`).toBeGreaterThanOrEqual(0);
    return layer.slice(layer.indexOf('{', at) + 1, layer.indexOf('}', at));
  };

  it('is layered, so a caller’s border-border-danger still wins over the resting look', () => {
    expect(CSS).toContain(OPEN);
    expect(CSS).not.toMatch(/\n\.br-field[ ,]/);
  });

  it('layers the Textarea defaults too, so a caller’s resize-none or min-h still wins', () => {
    // Inside the same `@layer components` block as `.br-field`, not a rule of its own.
    expect(CSS).not.toMatch(/\n\.br-textarea \{/);
    const rule = body('.br-textarea');
    expect(rule).toMatch(/min-block-size:\s*calc\(3 \* /);
    expect(rule).toMatch(/resize:\s*vertical/);
  });

  it('puts Input and Textarea on .br-field with a kept border and no line or ground of their own', () => {
    render(
      <>
        <Input aria-label="Name" />
        <Textarea aria-label="Notes" />
      </>
    );
    for (const name of ['Name', 'Notes']) {
      const field = screen.getByRole('textbox', { name });
      expect(field).toHaveClass('br-field', 'border');
      // A ground or an edge utility here would out-rank the layered look.
      expect(field.className).not.toMatch(
        /\bbg-background-|border-border-(?!danger)|inset-ring|disabled:bg-/
      );
    }
  });

  it('authors the rest fill, the hover step, and the focus and invalid edges together', () => {
    expect(body('.br-field')).toMatch(/border-color:\s*transparent/);
    expect(body('.br-field,\n.br-fill-rest')).toMatch(
      /background-color:\s*var\(--br-control-ground, var\(--background-fill\)\)/
    );
    expect(
      body('.br-field:hover:not(:focus, :disabled),\n.br-fill-hover:hover,\n.br-fill-hover:active')
    ).toMatch(/background-color:\s*var\(--background-fill-strong\)/);
    expect(body('.br-field:focus,\n.br-field:focus-visible')).toMatch(
      /border-color:\s*var\(--border-focus\)/
    );
    expect(body(".br-field[aria-invalid='true']")).toMatch(
      /border-color:\s*var\(--border-danger\)/
    );
    // The invalid edge comes after the focus edge, so an invalid field keeps it while focused.
    expect(layer.indexOf(".br-field[aria-invalid='true']")).toBeGreaterThan(
      layer.indexOf('.br-field:focus,')
    );
  });
});

/**
 * R-27: the secondary action is a subtle fill, not an outline. `outline` survives as a name for
 * the same box, with its 1px border kept transparent so nothing that used it reflows. Both step
 * between the two fill tokens (`.br-fill-rest`, `.br-fill-hover`) rather than taking an ink
 * overlay, because inside a dark popover the strong step is re-pointed DEEPER than the ground and
 * an overlay would lighten the button into it.
 */
describe('secondary and outline buttons are filled', () => {
  const CSS = readFileSync(resolve(__dirname, '../../styles/main.css'), 'utf8');
  const layer = CSS.slice(CSS.indexOf('@layer components {\n  .br-field,\n  .br-fill-rest {'));

  it.each(['secondary', 'outline'] as const)('draws %s as the subtle fill', (variant) => {
    render(<Button variant={variant}>Cancel</Button>);
    const button = screen.getByRole('button', { name: 'Cancel' });
    expect(button).toHaveClass('br-fill-rest', 'br-fill-hover', 'biorouter-focus-surface');
    expect(button.className).not.toMatch(
      /border-border-|bg-background-|hover:bg-|tint-interactive/
    );
  });

  it('authors the rest on the control ground and the hover and press on the strong step', () => {
    expect(layer.length, 'the layered fill rules exist').toBeGreaterThan(0);
    expect(layer).toMatch(
      /\.br-field,\n {2}\.br-fill-rest \{\n {4}background-color: var\(--br-control-ground, var\(--background-fill\)\);/
    );
    expect(layer).toMatch(
      /\.br-fill-hover:hover,\n {2}\.br-fill-hover:active \{\n {4}background-color: var\(--background-fill-strong\);/
    );
    // Inside a filled box the controls step back to the page ground, through a private variable:
    // the shared fill tokens are declared once per mode (themeNeutrals.test.ts).
    expect(CSS).toMatch(
      /\.br-fill-surface > \*,\n\.biorouter-modal-panel > \*,\n\.biorouter-modal-row > \* \{\n {2}--br-control-ground: var\(--background-default\);\n\}/
    );
  });

  it('keeps outline’s border width as a transparent edge, and gives secondary none', () => {
    render(
      <>
        <Button variant="outline">Outline</Button>
        <Button variant="secondary">Secondary</Button>
      </>
    );
    expect(screen.getByRole('button', { name: 'Outline' })).toHaveClass(
      'border',
      'border-transparent'
    );
    expect(screen.getByRole('button', { name: 'Secondary' }).className.split(/\s+/)).not.toContain(
      'border'
    );
  });

  it('lets a caller’s own ground win', () => {
    render(
      <Button variant="secondary" className="bg-background-default">
        Inside
      </Button>
    );
    const button = screen.getByRole('button', { name: 'Inside' });
    expect(button).toHaveClass('bg-background-default');
    expect(button).not.toHaveClass('bg-background-fill');
  });
});

describe('the compact button draws a 14px glyph', () => {
  it('names the rung for main.css, which sizes an unsized svg to 14px', () => {
    const css = readFileSync(resolve(__dirname, '../../styles/main.css'), 'utf8');
    render(
      <Button size="xs" aria-label="Copy">
        <svg />
      </Button>
    );
    expect(screen.getByRole('button', { name: 'Copy' })).toHaveClass('br-button-xs');
    expect(css).toMatch(
      /\.br-button-xs svg:not\(\[class\*='size-'\]\) \{\s*inline-size:\s*14px;\s*block-size:\s*14px;/
    );
  });
});
