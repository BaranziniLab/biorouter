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

  it('wraps in .br-form-field, never the text field’s own .br-field edge class', () => {
    const { container } = render(
      <Field id="wf-name" label="Name">
        <Input />
      </Field>
    );
    expect(container.querySelector('[data-slot="field"]')).toHaveClass('br-form-field');
    expect(container.querySelector('[data-slot="field"]')).not.toHaveClass('br-field');
    expect(screen.getByRole('textbox', { name: 'Name' })).toHaveClass('br-field');
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

describe('Field info name (vision V3-R3-3)', () => {
  /** The workflow editor's "Initial prompt (optional)": a component that draws a muted suffix. */
  function OptionalLabel({ children }: { children: string }) {
    return (
      <>
        {children} <span className="text-text-muted">(optional)</span>
      </>
    );
  }

  it('names the InfoTip by the words a node label shows, never by the field id', () => {
    render(
      <Field
        id="workflow-prompt"
        label={<OptionalLabel>Initial prompt</OptionalLabel>}
        info="Sent as the first message."
      >
        <Textarea />
      </Field>
    );
    expect(screen.getByRole('button', { name: 'About Initial prompt' })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /workflow-prompt/ })).toBeNull();
  });

  it('reads nested elements and collapses their whitespace', () => {
    render(
      <Field
        id="wf-model"
        label={
          <span>
            Model <strong>name</strong>
          </span>
        }
        info="Runs use this model."
      >
        <Input />
      </Field>
    );
    expect(screen.getByRole('button', { name: 'About Model name' })).toBeInTheDocument();
  });

  it('takes an explicit infoLabel over the derived words', () => {
    render(
      <Field
        id="workflow-prompt"
        label={<OptionalLabel>Initial prompt</OptionalLabel>}
        infoLabel="The first message"
        info="Sent as the first message."
      >
        <Textarea />
      </Field>
    );
    expect(screen.getByRole('button', { name: 'About The first message' })).toBeInTheDocument();
  });

  it('falls back to words, not the id, when the label node holds no text', () => {
    render(
      <Field id="wf-icon" label={<svg aria-hidden="true" />} info="Shown beside the name.">
        <Input />
      </Field>
    );
    expect(screen.getByRole('button', { name: 'About this field' })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /wf-icon/ })).toBeNull();
  });
});
