import { describe, it, expect, vi, beforeAll, afterAll } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { WorkflowResourcePicker } from '../WorkflowResourcePicker';

function renderPicker({
  selectedIds,
  defaultId,
}: {
  selectedIds: string[];
  defaultId: string | null;
}) {
  const onSelectedIdsChange = vi.fn();
  const onDefaultIdChange = vi.fn();
  render(
    <WorkflowResourcePicker
      label="Knowledge bases"
      items={[
        { id: 'lab-notes', label: 'lab-notes' },
        { id: 'soul', label: 'soul' },
      ]}
      selectedIds={selectedIds}
      onSelectedIdsChange={onSelectedIdsChange}
      defaultId={defaultId}
      onDefaultIdChange={onDefaultIdChange}
      emptyText="No knowledge bases found"
      searchPlaceholder="Search knowledge bases..."
      noun="KB"
    />
  );
  return { onSelectedIdsChange, onDefaultIdChange };
}

/**
 * The knowledge-base picker's default becomes the workflow's `default`, which
 * every chat the workflow starts takes as its primary: the target of KB-less
 * writes. The daemon never infers that pointer (`plan_knowledge_selection`),
 * so the picker does not either. Switching a base on or off changes which bases
 * the workflow can search, and only the Default control changes the default.
 */
describe('WorkflowResourcePicker default', () => {
  beforeAll(() => {
    // The picker is a Radix popover, and floating-ui measures it.
    vi.stubGlobal(
      'ResizeObserver',
      class {
        observe() {}
        unobserve() {}
        disconnect() {}
      }
    );
  });

  afterAll(() => {
    vi.unstubAllGlobals();
  });

  it('is not set by switching a base on', async () => {
    const user = userEvent.setup();
    const { onSelectedIdsChange, onDefaultIdChange } = renderPicker({
      selectedIds: [],
      defaultId: null,
    });

    await user.click(screen.getByText('No KBs selected'));
    await user.click(screen.getByRole('switch', { name: 'Toggle soul' }));

    expect(onSelectedIdsChange).toHaveBeenCalledWith(['soul']);
    expect(onDefaultIdChange).not.toHaveBeenCalled();
  });

  it('is cleared, not handed to another base, when its base is switched off', async () => {
    const user = userEvent.setup();
    const { onSelectedIdsChange, onDefaultIdChange } = renderPicker({
      selectedIds: ['lab-notes', 'soul'],
      defaultId: 'lab-notes',
    });

    await user.click(screen.getByText('2 KBs selected'));
    await user.click(screen.getByRole('switch', { name: 'Toggle lab-notes' }));

    expect(onSelectedIdsChange).toHaveBeenCalledWith(['soul']);
    expect(onDefaultIdChange).toHaveBeenCalledTimes(1);
    expect(onDefaultIdChange).toHaveBeenCalledWith(null);
  });

  it('is left alone when another base is switched off', async () => {
    const user = userEvent.setup();
    const { onSelectedIdsChange, onDefaultIdChange } = renderPicker({
      selectedIds: ['lab-notes', 'soul'],
      defaultId: 'lab-notes',
    });

    await user.click(screen.getByText('2 KBs selected'));
    await user.click(screen.getByRole('switch', { name: 'Toggle soul' }));

    expect(onSelectedIdsChange).toHaveBeenCalledWith(['lab-notes']);
    expect(onDefaultIdChange).not.toHaveBeenCalled();
  });

  // Without a way to take the default back, "these bases, and no default" —
  // what a chat with no primary captures — is gone after one click.
  it('is named by the Default control, and cleared by pressing it again', async () => {
    const user = userEvent.setup();
    const { onDefaultIdChange } = renderPicker({
      selectedIds: ['lab-notes', 'soul'],
      defaultId: 'lab-notes',
    });

    await user.click(screen.getByText('2 KBs selected'));
    const current = screen.getByRole('button', { name: 'Default KB: lab-notes' });
    const other = screen.getByRole('button', { name: 'Default KB: soul' });
    expect(current).toHaveAttribute('aria-pressed', 'true');
    expect(other).toHaveAttribute('aria-pressed', 'false');

    await user.click(other);
    expect(onDefaultIdChange).toHaveBeenLastCalledWith('soul');

    await user.click(current);
    expect(onDefaultIdChange).toHaveBeenLastCalledWith(null);
  });
});
