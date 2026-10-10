import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { ContextsSection } from './ContextsSection';
import { CONTEXTS, contextConfigKey } from './contexts';

const mockUpsert = vi.fn();
let mockConfig: Record<string, unknown> = {};

vi.mock('../../ConfigContext', () => ({
  useConfig: () => ({ config: mockConfig, upsert: mockUpsert }),
}));

describe('ContextsSection', () => {
  beforeEach(() => {
    mockConfig = {};
    mockUpsert.mockReset();
    mockUpsert.mockResolvedValue(undefined);
  });

  // Default ON, and absence means on: these load today and always have.
  it('shows every context switched on until a stored value says otherwise', () => {
    render(<ContextsSection />);
    expect(screen.getAllByRole('switch')).toHaveLength(CONTEXTS.length);
    for (const { label } of CONTEXTS) {
      expect(screen.getByRole('switch', { name: label })).toBeChecked();
    }
  });

  it('names each switch by its row label and describes it with the row help', () => {
    const { container } = render(<ContextsSection />);
    for (const { label, description } of CONTEXTS) {
      expect(screen.getByRole('switch', { name: label })).toHaveAccessibleDescription(description);
    }
    expect(screen.queryByRole('switch', { name: /toggle/i })).toBeNull();
    // One line per row: no description paragraph.
    expect(container.querySelectorAll('p')).toHaveLength(0);
  });

  it('says "loaded only for … tasks" in the section header, not on every office row', () => {
    for (const { description } of CONTEXTS) {
      expect(description).not.toMatch(/loaded only/i);
    }
  });

  it('reads a stored false as off and writes the plain config key on a toggle', async () => {
    mockConfig = { [contextConfigKey('office-pdf')]: false };
    render(<ContextsSection />);

    const pdf = screen.getByRole('switch', { name: 'PDF documents' });
    expect(pdf).not.toBeChecked();
    fireEvent.click(pdf);
    await waitFor(() =>
      expect(mockUpsert).toHaveBeenCalledWith(contextConfigKey('office-pdf'), true, false)
    );
  });
});
