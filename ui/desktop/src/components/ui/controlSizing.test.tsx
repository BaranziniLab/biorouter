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
 * Spec 2.3 / 2.6: every text field rests on `--border-control` (3:1), never the 1.6:1
 * `--border-emphasized`. The edge is authored (`.br-field`) and unlayered, so the same rule set
 * has to restate the focus and invalid edges or it would out-rank them.
 */
describe('text fields rest on the control edge', () => {
  const CSS = readFileSync(resolve(__dirname, '../../styles/main.css'), 'utf8');
  // The rules sit in `@layer components`, indented one level.
  const layer = CSS.slice(CSS.indexOf('@layer components {\n  .br-field {'));
  const body = (selector: string) => {
    const at = layer.indexOf(`\n  ${selector.replace(/\n/g, '\n  ')} {`);
    expect(at, `no layered rule for ${selector}`).toBeGreaterThanOrEqual(0);
    return layer.slice(layer.indexOf('{', at) + 1, layer.indexOf('}', at));
  };

  it('is layered, so a caller’s border-border-danger still wins over the resting edge', () => {
    expect(CSS).toContain('@layer components {\n  .br-field {');
    expect(CSS).not.toMatch(/\n\.br-field \{/);
  });

  it('layers the Textarea defaults too, so a caller’s resize-none or min-h still wins', () => {
    // Inside the same `@layer components` block as `.br-field`, not a rule of its own.
    expect(CSS).not.toMatch(/\n\.br-textarea \{/);
    const rule = body('.br-textarea');
    expect(rule).toMatch(/min-block-size:\s*calc\(3 \* /);
    expect(rule).toMatch(/resize:\s*vertical/);
  });

  it('puts Input and Textarea on .br-field with no faint edge utility', () => {
    render(
      <>
        <Input aria-label="Name" />
        <Textarea aria-label="Notes" />
      </>
    );
    for (const name of ['Name', 'Notes']) {
      const field = screen.getByRole('textbox', { name });
      expect(field).toHaveClass('br-field');
      expect(field.className).not.toMatch(/border-border-emphasized(?!\/)/);
    }
  });

  it('authors the resting, focus and invalid edges together', () => {
    expect(body('.br-field')).toMatch(/border-color:\s*var\(--border-control\)/);
    expect(body('.br-field:focus,\n.br-field:focus-visible')).toMatch(
      /border-color:\s*var\(--border-focus\)/
    );
    expect(body(".br-field[aria-invalid='true']")).toMatch(
      /border-color:\s*var\(--border-danger\)/
    );
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
