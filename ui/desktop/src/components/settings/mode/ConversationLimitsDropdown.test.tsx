import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
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
  beforeEach(() => {
    config.read.mockReset();
    config.upsert.mockReset();
  });

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

  it('never saves an empty or negative entry, and says why', async () => {
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
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
    // Saved once the typing settles, not on every keystroke (T3-SH-8).
    await waitFor(() => expect(onChange).toHaveBeenCalledWith(25));
    expect(onChange).toHaveBeenCalledTimes(1);
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

  // T3-SH-8: clearing the field by deleting one digit at a time saved each
  // digit left on the way ("10", then "1"), and each save's answer, arriving
  // after the field was already empty, wrote its number back into the field:
  // no message, and a value the person had just deleted.
  it('clearing the field leaves it empty, says why, and saves nothing on the way', async () => {
    config.read.mockImplementation(async (key: string) =>
      key === 'BIOROUTER_MAX_TURNS' ? 100 : 'auto'
    );
    let settle: () => void = () => {};
    config.upsert.mockImplementation(
      () =>
        new Promise<void>((resolve) => {
          settle = resolve;
        })
    );
    render(<ModeSection />);
    const field = openLimits();
    await waitFor(() => expect(field).toHaveValue(100));

    fireEvent.change(field, { target: { value: '10' } });
    fireEvent.change(field, { target: { value: '1' } });
    fireEvent.change(field, { target: { value: '' } });
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 600));
      settle();
    });

    expect(field).toHaveValue(null);
    expect(screen.getByRole('alert')).toHaveTextContent('Enter a whole number of at least 1.');
    expect(config.upsert).not.toHaveBeenCalled();
  });

  it("does not let a save's answer overwrite what was typed after it", async () => {
    config.read.mockImplementation(async (key: string) =>
      key === 'BIOROUTER_MAX_TURNS' ? 100 : 'auto'
    );
    const settles: Array<() => void> = [];
    config.upsert.mockImplementation(
      () =>
        new Promise<void>((resolve) => {
          settles.push(resolve);
        })
    );
    render(<ModeSection />);
    const field = openLimits();
    await waitFor(() => expect(field).toHaveValue(100));

    fireEvent.change(field, { target: { value: '40' } });
    await waitFor(() =>
      expect(config.upsert).toHaveBeenCalledWith('BIOROUTER_MAX_TURNS', 40, false)
    );
    fireEvent.change(field, { target: { value: '' } });
    await act(async () => {
      settles.forEach((resolve) => resolve());
    });

    expect(field).toHaveValue(null);
    expect(screen.getByRole('alert')).toHaveTextContent('Enter a whole number of at least 1.');
  });

  it('a save of the value already shown does not hide a later change from elsewhere', async () => {
    const onChange = vi.fn();
    const { rerender } = render(
      <ConversationLimitsDropdown maxTurns={40} onMaxTurnsChange={onChange} />
    );
    const field = openLimits();
    // Typed away and back inside one pause: one save, of the value shown.
    fireEvent.change(field, { target: { value: '4' } });
    fireEvent.change(field, { target: { value: '40' } });
    await waitFor(() => expect(onChange).toHaveBeenCalledWith(40));
    expect(onChange).toHaveBeenCalledTimes(1);

    // Another window moves it to 100 and back to 40: both are shown.
    rerender(<ConversationLimitsDropdown maxTurns={100} onMaxTurnsChange={onChange} />);
    expect(field).toHaveValue(100);
    rerender(<ConversationLimitsDropdown maxTurns={40} onMaxTurnsChange={onChange} />);
    expect(field).toHaveValue(40);
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
