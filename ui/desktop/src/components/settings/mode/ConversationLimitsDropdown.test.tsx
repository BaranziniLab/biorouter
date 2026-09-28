import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import {
  ConversationLimitsDropdown,
  DEFAULT_MAX_TURNS,
  parseMaxTurns,
} from './ConversationLimitsDropdown';
import { ModeSection } from './ModeSection';

const config = vi.hoisted(() => ({
  read: vi.fn(),
  upsert: vi.fn(),
}));

vi.mock('../../ConfigContext', () => ({
  useConfig: () => ({ read: config.read, upsert: config.upsert }),
}));

function openLimits() {
  fireEvent.click(screen.getByRole('button', { name: /chat limits/i }));
  return screen.getByRole('spinbutton', { name: 'Max turns' });
}

// W2-PRV-10: clearing the field saved 0 (`Number('')`), which stopped every new
// chat before its first model call; -5 was saved and ignored; a saved 0 was
// hidden behind the default; and the default shown was 1000 against the
// agent's 100.
describe('Max turns', () => {
  it('shows the agent default, read from the Rust constant it mirrors', () => {
    const agent = readFileSync(
      join(__dirname, '../../../../../../crates/biorouter/src/agents/agent.rs'),
      'utf8'
    );
    const match = agent.match(/pub const DEFAULT_MAX_TURNS: u32 = (\d+);/);
    expect(match).not.toBeNull();
    expect(DEFAULT_MAX_TURNS).toBe(Number(match![1]));

    render(<ConversationLimitsDropdown maxTurns={null} onMaxTurnsChange={vi.fn()} />);
    expect(openLimits()).toHaveValue(DEFAULT_MAX_TURNS);
  });

  it('accepts only a whole number of at least 1; empty is not 0', () => {
    for (const refused of ['', ' ', '0', '-5', '2.5', '1e3', 'abc', '4294967296']) {
      expect(parseMaxTurns(refused), JSON.stringify(refused)).toBeNull();
    }
    expect(parseMaxTurns('1')).toBe(1);
    expect(parseMaxTurns(' 250 ')).toBe(250);
  });

  it('never saves an empty or negative entry, and says why', () => {
    const onChange = vi.fn();
    render(<ConversationLimitsDropdown maxTurns={40} onMaxTurnsChange={onChange} />);
    const field = openLimits();

    fireEvent.change(field, { target: { value: '' } });
    expect(onChange).not.toHaveBeenCalled();
    expect(screen.getByRole('alert')).toHaveTextContent('Enter a whole number of at least 1.');
    expect(field).toHaveAttribute('aria-invalid', 'true');

    fireEvent.change(field, { target: { value: '-5' } });
    expect(onChange).not.toHaveBeenCalled();

    fireEvent.change(field, { target: { value: '0' } });
    expect(onChange).not.toHaveBeenCalled();

    fireEvent.change(field, { target: { value: '25' } });
    expect(onChange).toHaveBeenCalledWith(25);
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
  });

  it('shows a saved 0 as 0, so it can be seen and fixed', async () => {
    config.read.mockImplementation(async (key: string) =>
      key === 'BIOROUTER_MAX_TURNS' ? 0 : 'auto'
    );
    render(<ModeSection />);
    const field = openLimits();
    await waitFor(() => expect(field).toHaveValue(0));
    expect(screen.getByRole('alert')).toHaveTextContent(
      'The saved value, 0, is not a whole number of at least 1. Enter a new value.'
    );
  });

  it('saves a valid entry through the config', async () => {
    config.read.mockImplementation(async (key: string) =>
      key === 'BIOROUTER_MAX_TURNS' ? 30 : 'auto'
    );
    config.upsert.mockResolvedValue(undefined);
    render(<ModeSection />);
    const field = openLimits();
    await waitFor(() => expect(field).toHaveValue(30));
    fireEvent.change(field, { target: { value: '45' } });
    await waitFor(() =>
      expect(config.upsert).toHaveBeenCalledWith('BIOROUTER_MAX_TURNS', 45, false)
    );
    fireEvent.change(field, { target: { value: '' } });
    expect(config.upsert).toHaveBeenCalledTimes(1);
  });
});
