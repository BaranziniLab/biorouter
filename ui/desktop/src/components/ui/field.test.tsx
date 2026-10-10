import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { Field, fieldHelpId } from './field';
import { Input } from './input';
import { Textarea } from './textarea';

describe('Field', () => {
  it('labels the control and links the helper by aria-describedby', () => {
    render(
      <Field id="wf-name" label="Name" helper="Shown in the sidebar.">
        <Input />
      </Field>
    );
    const input = screen.getByRole('textbox', { name: 'Name' });
    expect(input).toHaveAttribute('id', 'wf-name');
    expect(input).toHaveAccessibleDescription('Shown in the sidebar.');
    expect(screen.getByText('Shown in the sidebar.')).toHaveAttribute('id', fieldHelpId('wf-name'));
  });

  it('replaces the helper with the error and marks the control invalid', () => {
    render(
      <Field id="wf-name" label="Name" helper="Shown in the sidebar." error="Name is taken">
        <Input />
      </Field>
    );
    const input = screen.getByRole('textbox', { name: 'Name' });
    expect(input).toHaveAttribute('aria-invalid', 'true');
    expect(input).toHaveAccessibleDescription('Name is taken');
    expect(screen.queryByText('Shown in the sidebar.')).toBeNull();
    expect(screen.getByText('Name is taken')).toHaveClass('text-text-danger');
  });
});

describe('Textarea', () => {
  it('is at least three rows, on the field recipe', () => {
    render(<Textarea aria-label="Notes" rows={1} />);
    const area = screen.getByRole('textbox', { name: 'Notes' });
    expect(area).toHaveAttribute('rows', '3');
    expect(area).toHaveClass('br-textarea', 'rounded-element', 'text-body');
  });
});

describe('Field info', () => {
  it('puts an InfoTip beside the label, never inside it, and describes the control with it', () => {
    render(
      <Field id="wf-model" label="Model" info="Runs use this model." helper="Required.">
        <Input />
      </Field>
    );
    const label = screen.getByText('Model');
    expect(label.tagName).toBe('LABEL');
    expect(label.querySelector('.br-info-tip')).toBeNull();
    expect(screen.getByRole('button', { name: 'About Model' })).toBeInTheDocument();
    const input = screen.getByRole('textbox', { name: 'Model' });
    expect(input.getAttribute('aria-describedby')?.split(' ')).toHaveLength(2);
    expect(input).toHaveAccessibleDescription('Runs use this model. Required.');
  });
});
