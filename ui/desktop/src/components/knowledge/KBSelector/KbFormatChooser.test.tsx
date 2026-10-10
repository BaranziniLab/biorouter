import { render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { KbFormatChooser } from './KbFormatChooser';
import { knowledgeCopy } from '../copy';

function renderChooser() {
  return render(
    <KbFormatChooser
      open
      onOpenChange={vi.fn()}
      initialName="Cohort notes"
      onCreate={vi.fn().mockResolvedValue('cohort-notes')}
    />
  );
}

/**
 * Spec 3.12: each format shows its name, its code and one short line; the facts
 * that used to sit under each option are its InfoTip. The folder preview is one
 * line with no dash, and the irreversibility notice is one warning Note.
 */
describe('KbFormatChooser', () => {
  it('shows one short line per format, with the facts as help', () => {
    renderChooser();
    for (const format of [knowledgeCopy.format.okf, knowledgeCopy.format.biookf]) {
      expect(screen.getByText(format.line)).toBeVisible();
      expect(
        screen.getByRole('button', { name: `About ${format.name}` })
      ).toHaveAccessibleDescription(format.info);
    }
    expect(screen.getAllByRole('radio')).toHaveLength(2);
  });

  it('previews the folder in one line, and says the format is permanent once', () => {
    renderChooser();
    expect(screen.getByText(knowledgeCopy.format.folder('cohort-notes'))).toBeVisible();
    expect(
      screen.getByRole('button', { name: `About ${knowledgeCopy.format.folderInfoLabel}` })
    ).toHaveAccessibleDescription(knowledgeCopy.format.folderInfo);
    expect(screen.getByText(knowledgeCopy.format.permanent)).toBeVisible();
    expect(screen.getByRole('dialog').textContent).not.toMatch(/[–—]/);
  });
});
