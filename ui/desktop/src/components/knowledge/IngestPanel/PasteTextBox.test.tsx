import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { knowledgeCopy } from '../copy';
import { PasteTextBox } from './PasteTextBox';

/**
 * V3-04: Digest is the Sources rail's one primary action. The box above it stages text, which is a
 * step toward Digest and not the goal, so "Stage" is a secondary button: with the default (coral)
 * variant two coral-filled buttons stood one above the other as soon as text was entered.
 */
describe('PasteTextBox', () => {
  it('stages with a secondary button, never a second primary', () => {
    const onStage = vi.fn();
    render(<PasteTextBox onStage={onStage} onCancel={vi.fn()} />);
    fireEvent.change(screen.getByPlaceholderText(knowledgeCopy.ingest.pastePlaceholder), {
      target: { value: 'some knowledge' },
    });

    const stage = screen.getByRole('button', { name: 'Stage' });
    expect(stage).toBeEnabled();
    // The primary variant is the one that paints the accent fill.
    expect(stage.className).not.toMatch(/bg-background-accent/);
    // The secondary is the filled button (WS-PRIMITIVES, R-27), not the old medium ground.
    expect(stage.className).toMatch(/\bbr-fill-rest\b/);
    expect(document.querySelectorAll('[class*="bg-background-accent"]')).toHaveLength(0);

    fireEvent.click(stage);
    expect(onStage).toHaveBeenCalledWith('some knowledge', 'Pasted knowledge', []);
  });

  // V3-R8-1: a "Will fetch" chip is a control inside the filled card, so it rests on the page
  // ground like the Title field and Stage (design.md 9.11 rule 3), not on the neutral badge's
  // strong fill, which was a fill on a fill. `pasteUrlChip.browser.test.tsx` measures it.
  it('rests a detected address chip on the page ground, and still toggles it', () => {
    const onStage = vi.fn();
    render(<PasteTextBox onStage={onStage} onCancel={vi.fn()} />);
    fireEvent.change(screen.getByPlaceholderText(knowledgeCopy.ingest.pastePlaceholder), {
      target: { value: 'See https://example.org/a and https://example.org/b' },
    });

    const chips = screen.getAllByRole('button', { pressed: true });
    expect(chips).toHaveLength(2);
    for (const chip of chips) {
      expect(chip.className).toMatch(/\bbg-background-default\b/);
      expect(chip.className).toMatch(/\bhover:bg-background-fill-strong\b/);
      expect(chip.className).toMatch(/\bbiorouter-focus-surface\b/);
    }

    fireEvent.click(chips[1]);
    expect(chips[1]).toHaveAttribute('aria-pressed', 'false');
    expect(chips[1].className).not.toMatch(/\btint-selected\b/);
    expect(chips[1].className).toMatch(/\bline-through\b/);

    fireEvent.click(screen.getByRole('button', { name: 'Stage' }));
    expect(onStage).toHaveBeenCalledWith(
      'See https://example.org/a and https://example.org/b',
      'Pasted knowledge',
      ['https://example.org/a']
    );
  });
});
