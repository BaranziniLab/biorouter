import { describe, it, expect, vi, beforeAll, afterAll } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { WorkflowResourcePicker } from '../WorkflowResourcePicker';
import { workflowFormCopy } from '../../copy';

function renderPicker({
  selectedIds,
  defaultId,
  noDefaultText,
  onDefaultIdChange = vi.fn(),
}: {
  selectedIds: string[];
  defaultId: string | null;
  noDefaultText?: string;
  onDefaultIdChange?: (id: string | null) => void;
}) {
  const onSelectedIdsChange = vi.fn();
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
      noDefaultText={noDefaultText}
      emptyText="No knowledge bases found"
      searchPlaceholder="Search knowledge bases…"
      noun="KB"
    />
  );
  return { onSelectedIdsChange, onDefaultIdChange };
}

const NO_DEFAULT = 'Chats it starts search every base above.';

/**
 * The knowledge-base picker's default becomes the workflow's `default`, which
 * every chat the workflow starts takes as its primary: the target of KB-less
 * writes. The daemon never infers that pointer (`plan_knowledge_selection`),
 * so the picker does not either. Switching a base on or off changes which bases
 * the workflow can search, and only the Default control changes the default.
 */
describe('WorkflowResourcePicker default', () => {
  // Set membership is a checkbox named by its item (spec 2.6), never a switch.
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
    await user.click(screen.getByRole('checkbox', { name: 'soul' }));

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
    await user.click(screen.getByRole('checkbox', { name: 'lab-notes' }));

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
    await user.click(screen.getByRole('checkbox', { name: 'soul' }));

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

/**
 * Whether a default is set is only ever *marked* on a row, and the rows live
 * inside a closed popover — so a captured "this chat has no primary base", which
 * is the correct capture for a chat that pinned none, looked exactly like a card
 * whose default had gone missing. The card has to say it.
 */
describe('WorkflowResourcePicker default summary', () => {
  it('puts what the picker is for behind an InfoTip, not under the label', () => {
    render(
      <WorkflowResourcePicker
        label="Skills"
        description="Skills this workflow prefers."
        items={[]}
        selectedIds={[]}
        onSelectedIdsChange={vi.fn()}
        emptyText="No skills found"
        searchPlaceholder="Search skills…"
        noun="skill"
      />
    );

    expect(screen.getByRole('button', { name: 'About Skills' })).toHaveAccessibleDescription(
      'Skills this workflow prefers.'
    );
    expect(screen.getByRole('group', { name: 'Skills' })).toBeInTheDocument();
  });

  it('says there is no default, on the card, without opening the popover', () => {
    renderPicker({
      selectedIds: ['lab-notes', 'soul'],
      defaultId: null,
      noDefaultText: NO_DEFAULT,
    });

    // The card says "No default" in words; what that costs is its InfoTip, one hover or
    // Tab away, and reachable by a screen reader through the glyph's description.
    const summary = screen.getByTestId('resource-picker-default-summary');
    expect(summary).toHaveTextContent(workflowFormCopy.picker.noDefault);
    expect(
      screen.getByRole('button', { name: `About ${workflowFormCopy.picker.noDefault}` })
    ).toHaveAccessibleDescription(NO_DEFAULT);
    // Still closed: the rows, and the Default control that marks one, are not
    // rendered at all.
    expect(screen.queryByRole('button', { name: 'Default KB: lab-notes' })).toBeNull();
  });

  it('names the default on the card when one is set', () => {
    renderPicker({
      selectedIds: ['lab-notes', 'soul'],
      defaultId: 'lab-notes',
      noDefaultText: NO_DEFAULT,
    });

    expect(screen.getByTestId('resource-picker-default-summary')).toHaveTextContent(
      'Default: lab-notes'
    );
  });

  // An empty selection already says so on the trigger, and "no default" on top
  // of it is a statement about a set with nothing in it.
  it('says nothing about a default when nothing is selected', () => {
    renderPicker({ selectedIds: [], defaultId: null, noDefaultText: NO_DEFAULT });

    expect(screen.queryByTestId('resource-picker-default-summary')).toBeNull();
  });

  // Skills and extensions have no default at all; the line must not appear for
  // a picker that cannot name one.
  it('says nothing for a picker with no default control', () => {
    render(
      <WorkflowResourcePicker
        label="Skills"
        items={[{ id: 'single-cell', label: 'single-cell' }]}
        selectedIds={['single-cell']}
        onSelectedIdsChange={vi.fn()}
        emptyText="No skills found"
        searchPlaceholder="Search skills…"
        noun="skill"
      />
    );

    expect(screen.queryByTestId('resource-picker-default-summary')).toBeNull();
  });
});
