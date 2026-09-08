import { cleanup, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import CustomSkillModal from './CustomSkillModal';

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
