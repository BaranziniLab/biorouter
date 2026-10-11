import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import JsonSchemaForm, { type JsonSchema } from './JsonSchemaForm';

afterEach(cleanup);

const schema: JsonSchema = {
  type: 'object',
  properties: {
    cohort: { type: 'string', title: 'Cohort', description: 'The cohort to analyse.' },
    sample_id: { type: 'string' },
    size: { type: 'integer', title: 'Sample size', minimum: 1 },
    region: { type: 'string', title: 'Region', enum: ['North', 'South'] },
    confirm: { type: 'boolean', title: 'Include controls', description: 'Adds the control arm.' },
    legacy: { type: 'boolean', description: 'Use the legacy pipeline' },
  },
  required: ['cohort'],
};

describe('JsonSchemaForm labels (WS-TOOLS-2)', () => {
  it('labels a field by its schema title, falling back to the key', () => {
    render(<JsonSchemaForm schema={schema} onSubmit={vi.fn()} />);
    expect(screen.getByLabelText(/^Cohort/)).toHaveAttribute('type', 'text');
    expect(screen.getByLabelText('Sample size')).toHaveAttribute('type', 'number');
    expect(screen.getByLabelText('Region').tagName).toBe('SELECT');
    // No title: the key is all there is.
    expect(screen.getByLabelText('sample_id')).toHaveAttribute('type', 'text');
    // The machine name never shows when a title exists.
    expect(screen.queryByText('cohort')).toBeNull();
    expect(screen.getByText('The cohort to analyse.')).toBeInTheDocument();
  });

  it('labels a checkbox by its title and keeps the description under it', () => {
    render(<JsonSchemaForm schema={schema} onSubmit={vi.fn()} />);
    expect(screen.getByLabelText('Include controls')).toHaveAttribute('type', 'checkbox');
    expect(screen.getByText('Adds the control arm.')).toBeInTheDocument();
    // An untitled checkbox still reads its description as its label.
    expect(screen.getByLabelText('Use the legacy pipeline')).toHaveAttribute('type', 'checkbox');
  });

  it('gives each form its own ids, so two cards asking for one key label their own fields', () => {
    render(
      <>
        <JsonSchemaForm schema={schema} onSubmit={vi.fn()} />
        <JsonSchemaForm schema={schema} onSubmit={vi.fn()} />
      </>
    );
    const fields = screen.getAllByLabelText(/^Cohort/);
    expect(fields).toHaveLength(2);
    expect(fields[0]).not.toBe(fields[1]);
    const ids = Array.from(document.querySelectorAll('[id]')).map((el) => el.id);
    expect(new Set(ids).size).toBe(ids.length);
    expect(ids).not.toContain('cohort');
  });
});
