import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { renderToString } from 'react-dom/server';
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

  /**
   * Effects do not run in a server render, so this is the FIRST paint: a stored "Expanded" has to
   * be drawn as Expanded right then, or the thumb is drawn at Collapsed and slides across on every
   * visit (a value read in an effect arrives one frame late).
   */
  it('draws the stored choice on the first render, not after an effect', () => {
    localStorage.setItem(RESPONSE_STYLE_STORAGE_KEY, 'detailed');
    const html = renderToString(<ToolCallDetailsRow />);
    const expandedButton = html.match(/<button[^>]*data-testid="tool-call-details-detailed"[^>]*>/);
    expect(expandedButton?.[0]).toMatch(/aria-checked="true"/);
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

describe('Spellcheck status', () => {
  const click = async (spellcheck: HTMLElement) => {
    await act(async () => {
      fireEvent.click(spellcheck);
    });
  };

  it('goes again when the switch is put back to what the window started with (V4-21)', async () => {
    render(<SpellcheckRow />);
    const spellcheck = await screen.findByRole('switch', { name: displayCopy.spellcheck });

    await click(spellcheck);
    expect(screen.getByText(displayCopy.restartToApply)).toBeInTheDocument();

    await click(spellcheck);
    expect(window.electron.setSpellcheck).toHaveBeenLastCalledWith(true);
    expect(screen.queryByText(displayCopy.restartToApply)).not.toBeInTheDocument();
    expect(spellcheck).toHaveAccessibleDescription(displayCopy.spellcheckHelp);

    // And it comes back for the next change, not only the first one.
    await click(spellcheck);
    expect(screen.getByText(displayCopy.restartToApply)).toBeInTheDocument();
  });

  it('measures from a stored OFF as well as from ON', async () => {
    window.electron.getSpellcheckState = vi.fn().mockResolvedValue(false);
    render(<SpellcheckRow />);
    const spellcheck = await screen.findByRole('switch', { name: displayCopy.spellcheck });
    await waitFor(() => expect(spellcheck).not.toBeChecked());
    expect(screen.queryByText(displayCopy.restartToApply)).not.toBeInTheDocument();

    await click(spellcheck);
    expect(spellcheck).toBeChecked();
    expect(screen.getByText(displayCopy.restartToApply)).toBeInTheDocument();

    await click(spellcheck);
    expect(screen.queryByText(displayCopy.restartToApply)).not.toBeInTheDocument();
  });
});

/**
 * The switch is the same control until the main process answers, held unpainted, and a later
 * visit starts from the last answer (vision round 8, V4-R7-2). A placeholder `true` drew the coral
 * ON track for a person who had turned spellcheck off, on every visit.
 */
describe('Spellcheck before and after the main process has answered', () => {
  it('holds the switch unpainted on a first visit rather than painting an ON it does not know', async () => {
    window.electron.getSpellcheckState = vi.fn(() => new Promise<boolean>(() => {}));
    render(<SpellcheckRow />);
    expect(document.querySelector('[data-slot="switch"]')).toHaveAttribute('data-pending');
    expect(screen.queryByRole('switch', { name: displayCopy.spellcheck })).toBeNull();
    expect(screen.queryByText(displayCopy.restartToApply)).not.toBeInTheDocument();
  });

  it('starts a later visit from the last answer, painted in the first render', async () => {
    window.electron.getSpellcheckState = vi.fn().mockResolvedValue(false);
    const first = render(<SpellcheckRow />);
    await waitFor(() =>
      expect(screen.getByRole('switch', { name: displayCopy.spellcheck })).not.toBeChecked()
    );
    first.unmount();

    window.electron.getSpellcheckState = vi.fn(() => new Promise<boolean>(() => {}));
    render(<SpellcheckRow />);
    expect(document.querySelector('[data-slot="switch"]')).not.toHaveAttribute('data-pending');
    expect(screen.getByRole('switch', { name: displayCopy.spellcheck })).not.toBeChecked();
    // Nothing is claimed about a restart before this row has heard from the app.
    expect(screen.queryByText(displayCopy.restartToApply)).not.toBeInTheDocument();
  });

  it('shows the default, on, when the state cannot be read and nothing was ever read', async () => {
    window.electron.getSpellcheckState = vi.fn().mockRejectedValue(new Error('no main process'));
    render(<SpellcheckRow />);
    expect(await screen.findByRole('switch', { name: displayCopy.spellcheck })).toBeChecked();
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
