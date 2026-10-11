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
});
