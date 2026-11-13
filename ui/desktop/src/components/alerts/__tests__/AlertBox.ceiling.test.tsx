import { describe, expect, it, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import { AlertBox } from '../AlertBox';
import { AlertType } from '../types';

// Each key reads its own value, as the real config does.
const values: Record<string, unknown> = { BIOROUTER_AUTO_COMPACT_THRESHOLD: 0.8 };
vi.mock('../../ConfigContext', () => ({
  useConfig: () => ({
    read: vi.fn(async (key: string) => values[key] ?? null),
    upsert: vi.fn().mockResolvedValue(undefined),
  }),
}));

const meter = (total: number) => (
  <AlertBox
    alert={{
      type: AlertType.Info,
      message: 'Context window',
      progress: { current: 100_000, total },
    }}
  />
);

describe('the context meter names where Biorouter really compacts', () => {
  it('says the token ceiling on a window over 250k tokens', async () => {
    render(meter(1_050_000));
    expect(await screen.findByText('Auto compact at 200k tokens')).toBeInTheDocument();
  });

  it('says the threshold where the ceiling does not reach', async () => {
    render(meter(128_000));
    expect(await screen.findByText('Auto compact at 80%')).toBeInTheDocument();
  });
});
