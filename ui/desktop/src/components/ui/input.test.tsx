import { createRef } from 'react';
import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { Input, machineStringProps } from './input';
import { SecretInput } from './secret-input';

/**
 * `<Input machine />` (vision V4-R7-5, shared by every form that holds a command, a URL, a model
 * id, a region, a variable or header name or a config value). The engine's side of this, that
 * Chromium stops drawing the red line, is read in `customProviderForm.browser.test.tsx` and
 * `capabilities/settingsBFill.browser.test.tsx`; jsdom has no spellchecker, so what is pinned
 * here is the markup the engine reads.
 */
describe('Input machine', () => {
  it('a plain field sets none of the three attributes, so a name or a description keeps its spellcheck', () => {
    render(<Input aria-label="Name" />);
    const field = screen.getByRole('textbox', { name: 'Name' });
    expect(field).not.toHaveAttribute('spellcheck');
    expect(field).not.toHaveAttribute('autocorrect');
    expect(field).not.toHaveAttribute('autocapitalize');
  });

  it('machine turns spellcheck, auto-correct and auto-capitalise off', () => {
    render(<Input machine aria-label="Command" />);
    const field = screen.getByRole('textbox', { name: 'Command' });
    expect(field).toHaveAttribute('spellcheck', 'false');
    expect(field).toHaveAttribute('autocorrect', 'off');
    expect(field).toHaveAttribute('autocapitalize', 'off');
  });

  it('machine={false} is a plain field', () => {
    render(<Input machine={false} aria-label="Name" />);
    expect(screen.getByRole('textbox', { name: 'Name' })).not.toHaveAttribute('spellcheck');
  });

  it('the prop is not written to the DOM', () => {
    render(<Input machine aria-label="Command" />);
    expect(screen.getByRole('textbox', { name: 'Command' })).not.toHaveAttribute('machine');
  });

  it('a caller’s own attribute still wins, so one field can opt back in', () => {
    render(<Input machine spellCheck aria-label="Command" />);
    const field = screen.getByRole('textbox', { name: 'Command' });
    expect(field).toHaveAttribute('spellcheck', 'true');
    expect(field).toHaveAttribute('autocorrect', 'off');
  });

  it('keeps the field’s own classes and forwards the ref', () => {
    const ref = createRef<HTMLInputElement>();
    render(<Input machine ref={ref} className="extra" aria-label="Command" />);
    const field = screen.getByRole('textbox', { name: 'Command' });
    expect(ref.current).toBe(field);
    expect(field).toHaveClass('br-field', 'extra');
  });

  it('the exported object is the same three attributes, for a props table built before the element', () => {
    expect(machineStringProps).toEqual({
      spellCheck: false,
      autoCorrect: 'off',
      autoCapitalize: 'off',
    });
  });

  it('a secret field has spellcheck off whatever it is given, with no machine prop', () => {
    render(<SecretInput revealLabel="API key" spellCheck aria-label="API key" />);
    expect(screen.getByLabelText('API key', { selector: 'input' })).toHaveAttribute(
      'spellcheck',
      'false'
    );
  });
});
