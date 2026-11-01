import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { DEFAULT_MAX_TURNS, MaxTurnsRow, parseMaxTurns } from './ConversationLimitsDropdown';
import { ModeSection } from './ModeSection';
import { approvalsCopy } from '../chat/copy';

const config = vi.hoisted(() => ({ read: vi.fn(), upsert: vi.fn() }));
vi.mock('../../ConfigContext', () => ({
  useConfig: () => ({ read: config.read, upsert: config.upsert }),
}));

const field = () => screen.getByRole('spinbutton', { name: 'Turns' });
const unlimited = () => screen.getByRole('radio', { name: 'Unlimited' });
const limited = () => screen.getByRole('radio', { name: 'Set a limit' });
const pause = () =>
  act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 500));
  });

function stored(value: number) {
  config.read.mockImplementation(async (key: string) =>
    key === 'BIOROUTER_MAX_TURNS' ? value : 'auto'
  );
}

describe('Max turns', () => {
  beforeEach(() => {
    config.read.mockReset();
    config.upsert.mockReset();
    config.upsert.mockResolvedValue(undefined);
  });

  it('mirrors the agent default without exposing the zero storage sentinel', () => {
    const agent = readFileSync(
      join(__dirname, '../../../../../../crates/biorouter/src/agents/agent.rs'),
      'utf8'
    );
    expect(DEFAULT_MAX_TURNS).toBe(
      Number(agent.match(/pub const DEFAULT_MAX_TURNS: u32 = (\d+);/)![1])
    );
    render(<MaxTurnsRow maxTurns={null} onMaxTurnsChange={vi.fn()} />);
    expect(
      screen.getByRole('radiogroup', { name: approvalsCopy.maxTurns })
    ).toHaveAccessibleDescription(approvalsCopy.maxTurnsHelp);
    expect(unlimited()).toBeChecked();
    expect(screen.queryByRole('spinbutton')).not.toBeInTheDocument();
  });

  it('keeps the storage parser compatible with zero and the full u32 range', () => {
    for (const refused of ['', ' ', '-5', '2.5', '1e3', 'abc', '4294967296']) {
      expect(parseMaxTurns(refused), JSON.stringify(refused)).toBeNull();
    }
    expect(parseMaxTurns('0')).toBe(0);
    expect(parseMaxTurns(' 0 ')).toBe(0);
    expect(parseMaxTurns('1')).toBe(1);
    expect(parseMaxTurns(' 4294967295 ')).toBe(4294967295);
  });

  it.each([null, 0])('shows saved %s as Unlimited without rewriting it', (value) => {
    const onChange = vi.fn();
    render(<MaxTurnsRow maxTurns={value} onMaxTurnsChange={onChange} />);
    expect(unlimited()).toBeChecked();
    expect(screen.queryByRole('spinbutton')).not.toBeInTheDocument();
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
    expect(onChange).not.toHaveBeenCalled();
  });

  it('holds both modes unselected and disabled until the saved setting is known', () => {
    const { rerender } = render(
      <MaxTurnsRow maxTurns={null} loaded={false} onMaxTurnsChange={vi.fn()} />
    );
    expect(unlimited()).toBeDisabled();
    expect(limited()).toBeDisabled();
    expect(unlimited()).not.toBeChecked();
    expect(limited()).not.toBeChecked();
    expect(screen.queryByRole('spinbutton')).not.toBeInTheDocument();
    rerender(<MaxTurnsRow maxTurns={25} loaded onMaxTurnsChange={vi.fn()} />);
    expect(limited()).toBeChecked();
    expect(field()).toHaveValue(25);
  });

  it('requires a chosen positive limit rather than inventing one', async () => {
    const onChange = vi.fn();
    render(<MaxTurnsRow maxTurns={null} onMaxTurnsChange={onChange} />);
    fireEvent.click(limited());
    expect(limited()).toBeChecked();
    expect(field()).toHaveValue(null);
    expect(field()).toHaveAttribute('aria-invalid', 'true');
    expect(field()).toHaveAccessibleDescription(
      `${approvalsCopy.maxTurnsHelp} Enter a whole number from 1 to 4,294,967,295.`
    );
    fireEvent.blur(field());
    await pause();
    expect(onChange).not.toHaveBeenCalled();
  });

  it.each(['', '0', '-5', '2.5', '1e3', '4294967296'])(
    'keeps invalid finite draft %j from changing the stored limit',
    async (value) => {
      const onChange = vi.fn();
      render(<MaxTurnsRow maxTurns={40} onMaxTurnsChange={onChange} />);
      fireEvent.change(field(), { target: { value } });
      fireEvent.blur(field());
      expect(screen.getByRole('alert')).toHaveTextContent(
        'Enter a whole number from 1 to 4,294,967,295.'
      );
      expect(limited()).toBeChecked();
      await pause();
      expect(onChange).not.toHaveBeenCalled();
    }
  );

  it.each([1, 4294967295])('saves valid finite boundary %s on blur', (value) => {
    const onChange = vi.fn();
    render(<MaxTurnsRow maxTurns={40} onMaxTurnsChange={onChange} />);
    fireEvent.change(field(), { target: { value: String(value) } });
    fireEvent.blur(field());
    expect(onChange).toHaveBeenCalledExactlyOnceWith(value);
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
  });

  it('keeps invalid saved numbers visible for correction', () => {
    render(<MaxTurnsRow maxTurns={-5} onMaxTurnsChange={vi.fn()} />);
    expect(limited()).toBeChecked();
    expect(field()).toHaveValue(-5);
    expect(screen.getByRole('alert')).toHaveTextContent(
      'The saved value, -5, is not a positive whole number. Enter a new value.'
    );
  });

  it('does not save intermediate digits while a limit is cleared', async () => {
    const onChange = vi.fn();
    render(<MaxTurnsRow maxTurns={100} onMaxTurnsChange={onChange} />);
    for (const value of ['10', '1', '']) fireEvent.change(field(), { target: { value } });
    await pause();
    expect(field()).toHaveValue(null);
    expect(onChange).not.toHaveBeenCalled();
  });

  it('cancels a pending finite save before a pointer selects Unlimited', async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    render(<MaxTurnsRow maxTurns={100} onMaxTurnsChange={onChange} />);
    await user.click(field());
    fireEvent.change(field(), { target: { value: '125' } });
    await user.click(unlimited());
    await pause();
    expect(onChange).toHaveBeenCalledExactlyOnceWith(0);
    expect(screen.queryByRole('spinbutton')).not.toBeInTheDocument();
    expect(unlimited()).toHaveFocus();
  });

  it('recovers the last positive draft when a limit is enabled again', () => {
    const onChange = vi.fn();
    render(<MaxTurnsRow maxTurns={40} onMaxTurnsChange={onChange} />);
    fireEvent.change(field(), { target: { value: '55' } });
    fireEvent.change(field(), { target: { value: '' } });
    fireEvent.click(unlimited());
    fireEvent.click(limited());
    expect(field()).toHaveValue(55);
    fireEvent.blur(field());
    expect(onChange.mock.calls).toEqual([[0], [55]]);
  });

  it('keeps Unlimited selected while an earlier ordered finite save is acknowledged', async () => {
    const onChange = vi.fn();
    const { rerender } = render(<MaxTurnsRow maxTurns={100} onMaxTurnsChange={onChange} />);
    fireEvent.change(field(), { target: { value: '40' } });
    fireEvent.blur(field());
    fireEvent.click(unlimited());
    expect(onChange.mock.calls).toEqual([[40], [0]]);
    rerender(<MaxTurnsRow maxTurns={40} onMaxTurnsChange={onChange} />);
    expect(unlimited()).toBeChecked();
    expect(screen.queryByRole('spinbutton')).not.toBeInTheDocument();
    rerender(<MaxTurnsRow maxTurns={0} onMaxTurnsChange={onChange} />);
    await pause();
    expect(onChange).toHaveBeenCalledTimes(2);
  });

  it('retires skipped save echoes so a later external finite value remains visible', () => {
    const onChange = vi.fn();
    const { rerender } = render(<MaxTurnsRow maxTurns={100} onMaxTurnsChange={onChange} />);
    for (const value of ['40', '60']) {
      fireEvent.change(field(), { target: { value } });
      fireEvent.blur(field());
    }
    fireEvent.click(unlimited());
    expect(onChange.mock.calls).toEqual([[40], [60], [0]]);
    rerender(<MaxTurnsRow maxTurns={0} onMaxTurnsChange={onChange} />);
    expect(unlimited()).toBeChecked();
    rerender(<MaxTurnsRow maxTurns={40} onMaxTurnsChange={onChange} />);
    expect(limited()).toBeChecked();
    expect(field()).toHaveValue(40);
    expect(onChange).toHaveBeenCalledTimes(3);
  });

  it('retires a repeated successful save whose value is already reflected', async () => {
    const settles: Array<() => void> = [];
    const onChange = vi.fn(
      () =>
        new Promise<void>((resolve) => {
          settles.push(resolve);
        })
    );
    const { rerender } = render(<MaxTurnsRow maxTurns={100} onMaxTurnsChange={onChange} />);
    fireEvent.change(field(), { target: { value: '40' } });
    fireEvent.blur(field());
    fireEvent.change(field(), { target: { value: '4' } });
    fireEvent.change(field(), { target: { value: '40' } });
    fireEvent.blur(field());
    await act(async () => {
      settles[0]();
    });
    rerender(<MaxTurnsRow maxTurns={40} onMaxTurnsChange={onChange} />);
    fireEvent.change(field(), { target: { value: '' } });
    await act(async () => {
      settles[1]();
    });
    expect(field()).toHaveValue(null);
    rerender(<MaxTurnsRow maxTurns={100} onMaxTurnsChange={onChange} />);
    expect(field()).toHaveValue(100);
    rerender(<MaxTurnsRow maxTurns={40} onMaxTurnsChange={onChange} />);
    expect(field()).toHaveValue(40);
  });

  it('retires coalesced successful echoes when the final value was already shown', async () => {
    const onChange = vi.fn(async () => {});
    const { rerender } = render(<MaxTurnsRow maxTurns={0} onMaxTurnsChange={onChange} />);
    fireEvent.click(limited());
    fireEvent.change(field(), { target: { value: '40' } });
    fireEvent.blur(field());
    fireEvent.click(unlimited());
    await act(async () => {});
    expect(onChange.mock.calls).toHaveLength(2);
    expect(unlimited()).toBeChecked();
    rerender(<MaxTurnsRow maxTurns={40} onMaxTurnsChange={onChange} />);
    expect(limited()).toBeChecked();
    expect(field()).toHaveValue(40);
  });

  it('retires through the latest completed duplicate when React coalesces acknowledgements', async () => {
    const onChange = vi.fn(async () => {});
    const { rerender } = render(<MaxTurnsRow maxTurns={100} onMaxTurnsChange={onChange} />);
    for (const value of ['40', '50', '40']) {
      fireEvent.change(field(), { target: { value } });
      fireEvent.blur(field());
    }
    fireEvent.change(field(), { target: { value: '' } });
    await act(async () => {});
    rerender(<MaxTurnsRow maxTurns={40} onMaxTurnsChange={onChange} />);
    expect(field()).toHaveValue(null);
    rerender(<MaxTurnsRow maxTurns={50} onMaxTurnsChange={onChange} />);
    expect(field()).toHaveValue(50);
  });

  it('keeps a later duplicate save pending when an earlier matching save is acknowledged', async () => {
    const settles: Array<() => void> = [];
    const onChange = vi.fn(
      () =>
        new Promise<void>((resolve) => {
          settles.push(resolve);
        })
    );
    const { rerender } = render(<MaxTurnsRow maxTurns={100} onMaxTurnsChange={onChange} />);
    for (const value of ['40', '50', '40']) {
      fireEvent.change(field(), { target: { value } });
      fireEvent.blur(field());
    }
    fireEvent.change(field(), { target: { value: '' } });
    for (const [index, value] of [40, 50, 40].entries()) {
      await act(async () => {
        settles[index]();
      });
      rerender(<MaxTurnsRow maxTurns={value} onMaxTurnsChange={onChange} />);
      expect(field()).toHaveValue(null);
    }
    rerender(<MaxTurnsRow maxTurns={50} onMaxTurnsChange={onChange} />);
    expect(field()).toHaveValue(50);
  });

  it('keeps newer typing when its own finite save returns', () => {
    const onChange = vi.fn();
    const { rerender } = render(<MaxTurnsRow maxTurns={100} onMaxTurnsChange={onChange} />);
    fireEvent.change(field(), { target: { value: '40' } });
    fireEvent.blur(field());
    fireEvent.change(field(), { target: { value: '' } });
    rerender(<MaxTurnsRow maxTurns={40} onMaxTurnsChange={onChange} />);
    expect(field()).toHaveValue(null);
    expect(screen.getByRole('alert')).toBeInTheDocument();
  });

  it('applies external updates and cancels an older unsaved draft', async () => {
    const onChange = vi.fn();
    const { rerender } = render(<MaxTurnsRow maxTurns={40} onMaxTurnsChange={onChange} />);
    fireEvent.change(field(), { target: { value: '55' } });
    rerender(<MaxTurnsRow maxTurns={70} onMaxTurnsChange={onChange} />);
    expect(field()).toHaveValue(70);
    await pause();
    expect(onChange).not.toHaveBeenCalled();
    rerender(<MaxTurnsRow maxTurns={0} onMaxTurnsChange={onChange} />);
    expect(unlimited()).toBeChecked();
    expect(screen.queryByRole('spinbutton')).not.toBeInTheDocument();
  });

  it('flushes a valid pending draft when the settings tab unmounts', () => {
    const onChange = vi.fn();
    const visit = render(<MaxTurnsRow maxTurns={40} onMaxTurnsChange={onChange} />);
    fireEvent.change(field(), { target: { value: '75' } });
    visit.unmount();
    expect(onChange).toHaveBeenCalledExactlyOnceWith(75);
  });

  it('keeps a same-value save from swallowing later external changes', () => {
    const onChange = vi.fn();
    const { rerender } = render(<MaxTurnsRow maxTurns={40} onMaxTurnsChange={onChange} />);
    fireEvent.change(field(), { target: { value: '4' } });
    fireEvent.change(field(), { target: { value: '40' } });
    fireEvent.blur(field());
    rerender(<MaxTurnsRow maxTurns={100} onMaxTurnsChange={onChange} />);
    expect(field()).toHaveValue(100);
    rerender(<MaxTurnsRow maxTurns={40} onMaxTurnsChange={onChange} />);
    expect(field()).toHaveValue(40);
  });

  it('supports one named keyboard choice followed by its conditional input', async () => {
    const user = userEvent.setup();
    render(<MaxTurnsRow maxTurns={0} onMaxTurnsChange={vi.fn()} />);
    act(() => unlimited().focus());
    await user.keyboard('{ArrowRight}');
    await waitFor(() => expect(limited()).toBeChecked());
    expect(limited()).toHaveFocus();
    await user.tab();
    expect(field()).toHaveFocus();
    await user.tab({ shift: true });
    await user.keyboard('{ArrowLeft}');
    await waitFor(() => expect(unlimited()).toBeChecked());
    expect(unlimited()).toHaveFocus();
    expect(screen.queryByRole('spinbutton')).not.toBeInTheDocument();
  });

  it('persists a finite value through the config after typing settles', async () => {
    stored(30);
    render(<ModeSection />);
    await waitFor(() => expect(field()).toHaveValue(30));
    expect(config.upsert).not.toHaveBeenCalled();
    fireEvent.change(field(), { target: { value: '45' } });
    await waitFor(() =>
      expect(config.upsert).toHaveBeenCalledExactlyOnceWith('BIOROUTER_MAX_TURNS', 45, false)
    );
  });

  it('orders actual config writes across a tab change so Unlimited remains saved', async () => {
    let saved = 100;
    config.read.mockImplementation(async (key: string) =>
      key === 'BIOROUTER_MAX_TURNS' ? saved : 'auto'
    );
    const settles: Array<() => void> = [];
    config.upsert.mockImplementation(async (_key: string, value: number) => {
      await new Promise<void>((resolve) => {
        settles.push(resolve);
      });
      saved = value;
    });
    const visit = render(<ModeSection />);
    await waitFor(() => expect(field()).toHaveValue(100));
    fireEvent.change(field(), { target: { value: '40' } });
    fireEvent.blur(field());
    await waitFor(() => expect(config.upsert).toHaveBeenCalledTimes(1));
    fireEvent.click(unlimited());
    expect(config.upsert).toHaveBeenCalledTimes(1);
    visit.unmount();
    render(<ModeSection />);
    await act(async () => {
      settles[0]();
    });
    await waitFor(() => expect(config.upsert).toHaveBeenCalledTimes(2));
    expect(saved).toBe(40);
    expect(config.upsert.mock.calls).toEqual([
      ['BIOROUTER_MAX_TURNS', 40, false],
      ['BIOROUTER_MAX_TURNS', 0, false],
    ]);
    await act(async () => {
      settles[1]();
    });
    await waitFor(() => expect(unlimited()).toBeChecked());
    expect(saved).toBe(0);
    expect(screen.queryByRole('spinbutton')).not.toBeInTheDocument();
  });

  it('keeps a newer blank finite draft while a queued same-value Unlimited save completes', async () => {
    stored(0);
    const settles: Array<() => void> = [];
    config.upsert.mockImplementation(
      () =>
        new Promise<void>((resolve) => {
          settles.push(resolve);
        })
    );
    render(<ModeSection />);
    await waitFor(() => expect(unlimited()).toBeChecked());
    fireEvent.click(limited());
    fireEvent.change(field(), { target: { value: '40' } });
    fireEvent.blur(field());
    await waitFor(() => expect(config.upsert).toHaveBeenCalledTimes(1));
    fireEvent.click(unlimited());
    fireEvent.click(limited());
    fireEvent.change(field(), { target: { value: '' } });
    await act(async () => {
      settles[0]();
    });
    await waitFor(() => expect(config.upsert).toHaveBeenCalledTimes(2));
    expect(limited()).toBeChecked();
    expect(field()).toHaveValue(null);
    await act(async () => {
      settles[1]();
    });
    expect(limited()).toBeChecked();
    expect(field()).toHaveValue(null);
    expect(screen.getByRole('alert')).toBeInTheDocument();
    expect(config.upsert.mock.calls).toEqual([
      ['BIOROUTER_MAX_TURNS', 40, false],
      ['BIOROUTER_MAX_TURNS', 0, false],
    ]);
  });

  it('discards a read that began before Unlimited was saved', async () => {
    stored(100);
    const first = render(<ModeSection />);
    await waitFor(() => expect(field()).toHaveValue(100));
    first.unmount();
    let saved = 100;
    let releaseRead: () => void = () => {};
    let reads = 0;
    config.read.mockImplementation(async (key: string) => {
      if (key !== 'BIOROUTER_MAX_TURNS') return 'auto';
      const snapshot = saved;
      if (reads++ === 0)
        await new Promise<void>((resolve) => {
          releaseRead = resolve;
        });
      return snapshot;
    });
    config.upsert.mockImplementation(async (_key: string, value: number) => {
      saved = value;
    });
    render(<ModeSection />);
    await act(async () => {});
    fireEvent.click(unlimited());
    await waitFor(() => expect(saved).toBe(0));
    await act(async () => {
      releaseRead();
    });
    await waitFor(() => expect(reads).toBe(2));
    expect(unlimited()).toBeChecked();
    expect(screen.queryByRole('spinbutton')).not.toBeInTheDocument();
  });

  it.each([0, 100])(
    'accepts an external finite limit after a failed save and an Unlimited save, starting at %s',
    async (initial) => {
      stored(initial);
      const first = render(<ModeSection />);
      if (initial === 0) await waitFor(() => expect(unlimited()).toBeChecked());
      else await waitFor(() => expect(field()).toHaveValue(initial));
      first.unmount();

      let saved = initial;
      let releaseRead: () => void = () => {};
      let reads = 0;
      config.read.mockImplementation(async (key: string) => {
        if (key !== 'BIOROUTER_MAX_TURNS') return 'auto';
        const snapshot = saved;
        if (reads++ === 0)
          await new Promise<void>((resolve) => {
            releaseRead = resolve;
          });
        return snapshot;
      });
      const logged = vi.spyOn(console, 'error').mockImplementation(() => {});
      config.upsert.mockImplementation(async (_key: string, value: number) => {
        if (value === 40) throw new Error('write refused');
        saved = value;
      });
      try {
        render(<ModeSection />);
        await act(async () => {});
        if (initial === 0) fireEvent.click(limited());
        fireEvent.change(field(), { target: { value: '40' } });
        fireEvent.blur(field());
        await waitFor(() => expect(logged).toHaveBeenCalledTimes(1));
        fireEvent.click(unlimited());
        await waitFor(() => expect(config.upsert).toHaveBeenCalledTimes(2));
        await waitFor(() => expect(saved).toBe(0));
        expect(unlimited()).toBeChecked();

        saved = 40;
        await act(async () => {
          releaseRead();
        });
        await waitFor(() => expect(reads).toBe(2));
        expect(limited()).toBeChecked();
        expect(field()).toHaveValue(40);
        expect(config.upsert.mock.calls).toEqual([
          ['BIOROUTER_MAX_TURNS', 40, false],
          ['BIOROUTER_MAX_TURNS', 0, false],
        ]);
      } finally {
        releaseRead();
        logged.mockRestore();
      }
    }
  );
});
