import { cleanup, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import CustomSkillModal from './CustomSkillModal';
import { CUSTOM_SKILL_COPY } from './copy';

afterEach(cleanup);

describe('CustomSkillModal', () => {
  it('blocks all dismissal while the skill is being written', async () => {
    const user = userEvent.setup();
    const onClose = vi.fn();
    let finishWrite: (value: boolean) => void = () => {};
    const pendingWrite = new Promise<boolean>((resolve) => {
      finishWrite = resolve;
    });

    (window as unknown as { electron: unknown }).electron = {
      ensureDirectory: vi.fn().mockResolvedValue(undefined),
      writeFile: vi.fn().mockReturnValue(pendingWrite),
    };

    render(<CustomSkillModal onClose={onClose} onSaved={() => {}} />);
    await user.click(screen.getByRole('button', { name: 'Save skill' }));

    expect(await screen.findByRole('button', { name: 'Saving…' })).toBeDisabled();
    expect(screen.queryByRole('button', { name: 'Close' })).not.toBeInTheDocument();
    await user.keyboard('{Escape}');
    expect(screen.getByRole('dialog')).toBeInTheDocument();
    expect(onClose).not.toHaveBeenCalled();

    finishWrite(false);
    await waitFor(() => expect(screen.getByRole('button', { name: 'Save skill' })).toBeEnabled());
  });
});

describe('CustomSkillModal: the guidance is help on demand', () => {
  it('shows no sentence, and gives the editor its requirement and its location as a description', () => {
    render(<CustomSkillModal onClose={() => {}} onSaved={() => {}} />);

    // Nothing but the label is visible: the requirement and the location are the help.
    expect(screen.queryByText(CUSTOM_SKILL_COPY.help, { selector: ':not(.sr-only)' })).toBeNull();
    const editor = screen.getByRole('textbox', { name: CUSTOM_SKILL_COPY.editorLabel });
    expect(editor).toHaveAccessibleDescription(CUSTOM_SKILL_COPY.help);
    expect(
      screen.getByRole('button', { name: `About ${CUSTOM_SKILL_COPY.editorLabel}` })
    ).toHaveAccessibleDescription(CUSTOM_SKILL_COPY.help);
  });

  it('puts first focus in the editor, not on the help glyph', async () => {
    render(<CustomSkillModal onClose={() => {}} onSaved={() => {}} />);
    await waitFor(() =>
      expect(screen.getByRole('textbox', { name: CUSTOM_SKILL_COPY.editorLabel })).toHaveFocus()
    );
  });

  it('is anchored at the top, so a refused save grows it downward', () => {
    render(<CustomSkillModal onClose={() => {}} onSaved={() => {}} />);
    expect(screen.getByRole('dialog')).toHaveAttribute('data-anchor', 'top');
    expect(screen.getByRole('dialog')).toHaveAttribute('data-size', 'lg');
  });

  it('says what is missing when the frontmatter is not there', async () => {
    const user = userEvent.setup();
    render(<CustomSkillModal onClose={() => {}} onSaved={() => {}} />);
    const editor = screen.getByRole('textbox', { name: CUSTOM_SKILL_COPY.editorLabel });
    await user.clear(editor);
    await user.type(editor, 'no frontmatter');
    await user.click(screen.getByRole('button', { name: CUSTOM_SKILL_COPY.save }));
    expect(await screen.findByRole('alert')).toHaveTextContent(CUSTOM_SKILL_COPY.invalid);
  });
});
