import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, fireEvent, render, screen } from '@testing-library/react';
import { displayCopy, projectCopy } from './copy';
import { SpellcheckRow } from './SpellcheckToggle';
import { ProjectHintsRow } from './BioRouterHintsSection';
import {
  RESPONSE_STYLE_STORAGE_KEY,
  ToolCallDetailsRow,
} from '../response_styles/ResponseStylesSection';

// The dialog has its own test; this file is about the row that opens it.
vi.mock('./BioRouterHintsModal', () => ({
  BioRouterHintsModal: () => <div role="dialog" aria-label="Project hints dialog" />,
}));

const [expanded, collapsed] = displayCopy.toolCallOptions;

beforeEach(() => {
  localStorage.clear();
  Object.assign(window, {
    electron: {
      getSpellcheckState: vi.fn().mockResolvedValue(true),
      setSpellcheck: vi.fn().mockResolvedValue(true),
    },
    appConfig: { get: vi.fn().mockReturnValue('/tmp/project') },
  });
});

afterEach(() => {
  localStorage.clear();
});

/**
 * Settings > Chat > Display and Project (spec §3.13): one row each, named by its visible label,
 * with the explanation as help the control hears rather than a paragraph.
 */
describe('Tool call details', () => {
  it('is one segmented choice named by its row, Collapsed by default and stored as concise', () => {
    render(<ToolCallDetailsRow />);
    const group = screen.getByRole('radiogroup', { name: displayCopy.toolCallDetails });
    expect(group).toHaveAccessibleDescription(displayCopy.toolCallDetailsHelp);
    expect(screen.getByRole('radio', { name: collapsed.label })).toHaveAttribute(
      'aria-checked',
      'true'
    );
    expect(localStorage.getItem(RESPONSE_STYLE_STORAGE_KEY)).toBe('concise');
  });

  it('shows the stored choice, and keeps the stored keys when the person picks the other', () => {
    localStorage.setItem(RESPONSE_STYLE_STORAGE_KEY, 'detailed');
    const heard = vi.fn();
    window.addEventListener('responseStyleChanged', heard);
    render(<ToolCallDetailsRow />);
    expect(screen.getByRole('radio', { name: expanded.label })).toHaveAttribute(
      'aria-checked',
      'true'
    );

    fireEvent.click(screen.getByRole('radio', { name: collapsed.label }));
    expect(localStorage.getItem(RESPONSE_STYLE_STORAGE_KEY)).toBe('concise');
    // The transcript listens for this event to re-read the choice.
    expect(heard).toHaveBeenCalledTimes(1);
    window.removeEventListener('responseStyleChanged', heard);
  });
});

describe('Spellcheck', () => {
  it('is a switch named by its row, with no status line until it is changed', async () => {
    render(<SpellcheckRow />);
    const spellcheck = await screen.findByRole('switch', { name: displayCopy.spellcheck });
    expect(spellcheck).toHaveAccessibleDescription(displayCopy.spellcheckHelp);
    expect(screen.queryByText(displayCopy.restartToApply)).not.toBeInTheDocument();
  });

  it('says "Restart to apply" after a change, and the switch hears it', async () => {
    render(<SpellcheckRow />);
    const spellcheck = await screen.findByRole('switch', { name: displayCopy.spellcheck });
    await act(async () => {
      fireEvent.click(spellcheck);
    });
    expect(window.electron.setSpellcheck).toHaveBeenCalledWith(false);
    expect(screen.getByText(displayCopy.restartToApply)).toBeInTheDocument();
    expect(spellcheck).toHaveAccessibleDescription(
      `${displayCopy.spellcheckHelp} ${displayCopy.restartToApply}`
    );
  });
});

describe('Project hints', () => {
  it('is one Edit… button whose name says what it edits, with the filename in its help', () => {
    render(<ProjectHintsRow />);
    const edit = screen.getByRole('button', { name: projectCopy.editHints });
    expect(edit).toHaveTextContent(projectCopy.edit);
    expect(edit).toHaveAccessibleDescription(projectCopy.hintsHelp);
    expect(projectCopy.hintsHelp).toContain('.biorouterhints');
    // The filename left the label for the InfoTip.
    expect(screen.queryByText(/\(\.biorouterhints\)/)).not.toBeInTheDocument();
  });

  it('opens the hints dialog from the row', () => {
    render(<ProjectHintsRow />);
    fireEvent.click(screen.getByRole('button', { name: projectCopy.editHints }));
    expect(screen.getByRole('dialog', { name: 'Project hints dialog' })).toBeInTheDocument();
  });
});
