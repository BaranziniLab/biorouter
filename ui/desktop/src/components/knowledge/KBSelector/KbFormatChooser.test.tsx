import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi, type Mock } from 'vitest';
import type { KbFormat } from '../../../api/types.gen';
import { KbFormatChooser } from './KbFormatChooser';
import { knowledgeCopy } from '../copy';

type Create = (name: string, format: KbFormat) => Promise<string | undefined>;

function renderChooser(props: { initialName?: string; onCreate?: Mock<Create> } = {}) {
  const onCreate = props.onCreate ?? vi.fn<Create>().mockResolvedValue('cohort-notes');
  render(
    <KbFormatChooser
      open
      onOpenChange={vi.fn()}
      initialName={props.initialName ?? 'Cohort notes'}
      onCreate={onCreate}
    />
  );
  return { onCreate };
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

/**
 * V3-R4-3. An empty Name used to put a detached 14px paragraph at the foot of the dialog: no
 * `aria-invalid`, no danger edge, focus left on the button, and its height pushed Cancel and Create
 * down from under the pointer. A name problem is the Name field's error now.
 */
describe('KbFormatChooser, a name that cannot be used', () => {
  const nameField = () => screen.getByTestId('knowledge-format-name') as HTMLInputElement;

  it('says it on the field, takes the folder line’s place, and moves focus to the field', async () => {
    const user = userEvent.setup();
    const { onCreate } = renderChooser({ initialName: '' });
    // Focus is on the button, as it is after a click on Create.
    await user.click(screen.getByTestId('knowledge-format-submit'));

    const alert = await screen.findByRole('alert');
    expect(alert).toHaveTextContent(knowledgeCopy.format.nameRequired);
    expect(onCreate).not.toHaveBeenCalled();

    const field = nameField();
    expect(field).toHaveAttribute('aria-invalid', 'true');
    expect(field).toHaveAttribute('aria-required', 'true');
    expect(field).toHaveAccessibleDescription(knowledgeCopy.format.nameRequired);
    expect(field).toHaveFocus();

    // It is the field's own line (12px `text-supporting`, danger ink), not a body paragraph.
    const line = alert.closest('[data-slot="field-error"]');
    expect(line).not.toBeNull();
    expect(line!.className).toMatch(/\btext-supporting\b/);
    expect(line!.className).toMatch(/\btext-text-danger\b/);
    expect(line!.className).not.toMatch(/\btext-body\b/);

    // It replaces the folder preview rather than adding a line (the dialog does not grow).
    expect(screen.queryByText(/^knowledge\//)).toBeNull();
    expect(screen.getAllByRole('alert')).toHaveLength(1);
  });

  it('does the same from Enter in the field, and clears and restores the preview when typing', async () => {
    const user = userEvent.setup();
    renderChooser({ initialName: '' });
    await user.type(nameField(), '{Enter}');
    expect(await screen.findByRole('alert')).toHaveTextContent(knowledgeCopy.format.nameRequired);
    expect(nameField()).toHaveFocus();

    await user.type(nameField(), 'A');
    expect(screen.queryByRole('alert')).toBeNull();
    expect(nameField()).not.toHaveAttribute('aria-invalid');
    expect(screen.getByText(knowledgeCopy.format.folder('a'))).toBeVisible();
  });

  it('refuses a name with no letters or numbers the same way', async () => {
    const user = userEvent.setup();
    const { onCreate } = renderChooser({ initialName: '' });
    fireEvent.change(nameField(), { target: { value: '!!!' } });
    await user.click(screen.getByTestId('knowledge-format-submit'));
    expect(await screen.findByRole('alert')).toHaveTextContent(
      knowledgeCopy.format.nameNeedsLetters
    );
    expect(nameField()).toHaveAttribute('aria-invalid', 'true');
    expect(nameField()).toHaveFocus();
    expect(onCreate).not.toHaveBeenCalled();
  });

  it('keeps a failure the daemon reports off the field, as a danger note at the foot', async () => {
    const user = userEvent.setup();
    const { onCreate } = renderChooser({
      onCreate: vi.fn<Create>().mockRejectedValue(new Error('The daemon is not reachable.')),
    });
    await user.click(screen.getByTestId('knowledge-format-submit'));
    await waitFor(() => expect(onCreate).toHaveBeenCalled());

    const alert = await screen.findByRole('alert');
    expect(alert).toHaveTextContent('The daemon is not reachable.');
    // Not about the name, so the field is not marked invalid and its folder line stays.
    expect(nameField()).not.toHaveAttribute('aria-invalid');
    expect(screen.getByText(knowledgeCopy.format.folder('cohort-notes'))).toBeVisible();
    expect(alert.closest('[data-slot="field"]')).toBeNull();
  });
});
