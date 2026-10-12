import { describe, it, expect, vi, beforeEach } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { BrsdkSection } from './BrsdkSection';
import { BRSDK_TOGGLES } from './copy';

const mockUpsert = vi.fn();
let mockConfig: Record<string, unknown> = {};

vi.mock('../../ConfigContext', () => ({
  useConfig: () => ({
    config: mockConfig,
    upsert: mockUpsert,
  }),
}));

// A switch is named by its row's visible label (spec 2.6): no "Toggle …".
const switchFor = (label: string) => screen.getByRole('switch', { name: label });

describe('BrsdkSection', () => {
  beforeEach(() => {
    mockConfig = {};
    mockUpsert.mockReset();
    mockUpsert.mockResolvedValue(undefined);
  });

  it('defaults every App SDK safety framework switch off', () => {
    render(<BrsdkSection />);

    expect(screen.getAllByRole('switch')).toHaveLength(BRSDK_TOGGLES.length);
    for (const { label } of BRSDK_TOGGLES) {
      const toggle = switchFor(label);
      expect(toggle).toHaveAttribute('aria-checked', 'false');
      expect(toggle.closest('.biorouter-settings-row')).toHaveClass('px-3', 'py-2.5');
    }
  });

  it('describes each switch with its row help instead of printing it under the label', () => {
    const { container } = render(<BrsdkSection />);

    for (const { label, help } of BRSDK_TOGGLES) {
      expect(switchFor(label)).toHaveAccessibleDescription(help);
    }
    expect(container.querySelectorAll('p')).toHaveLength(0);
  });

  it('writes Agent Drafter without a hyphen and stop hook in lower case', () => {
    const prose = BRSDK_TOGGLES.map(({ help }) => help).join(' ');
    expect(prose).toMatch(/Agent Drafter/);
    expect(prose).not.toMatch(/Agent-Drafter/);
    expect(prose).not.toMatch(/Stop-hook/);
  });

  it('reflects persisted true values independently', () => {
    mockConfig = {
      brsdk_pii_guardrail: true,
      brsdk_tracing: true,
    };

    render(<BrsdkSection />);

    expect(switchFor('PII / PHI guardrail')).toHaveAttribute('aria-checked', 'true');
    expect(switchFor('Goal stop-hook guardrail')).toHaveAttribute('aria-checked', 'false');
    expect(switchFor('Encrypted vault')).toHaveAttribute('aria-checked', 'false');
    expect(switchFor('Agent tracing')).toHaveAttribute('aria-checked', 'true');
  });

  it('writes each backend config key as a non-secret setting', async () => {
    render(<BrsdkSection />);

    for (const { label, key } of BRSDK_TOGGLES) {
      fireEvent.click(switchFor(label));
      await waitFor(() => {
        expect(mockUpsert).toHaveBeenCalledWith(key, true, false);
      });
    }
  });
});
