import { render, screen } from '@testing-library/react';
import { createRef } from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { KBSelectorTrigger } from './KBSelectorTrigger';
import { knowledgeCopy } from '../copy';
import type { Manifest } from '../../../api/types.gen';

const state = vi.hoisted(() => ({
  primaryKb: null as Partial<Manifest> | null,
  visibleBases: [] as Partial<Manifest>[],
}));

vi.mock('../KnowledgeContext', () => ({
  useKnowledge: () => state,
}));

vi.mock('./KBSelectorMenu', () => ({
  KBSelectorMenu: () => null,
}));

function base(id: string) {
  return { id, name: id, color: '#cf6d47' };
}

beforeEach(() => {
  state.primaryKb = null;
  state.visibleBases = [];
});

describe('KBSelectorTrigger', () => {
  it('names the primary and counts the rest of the set', () => {
    state.primaryKb = base('alpha');
    state.visibleBases = [base('alpha'), base('beta'), base('gamma')];
    render(<KBSelectorTrigger onManage={() => {}} onCreate={() => {}} />);
    expect(screen.getByText('alpha')).toBeInTheDocument();
    expect(screen.getByText('+2')).toBeInTheDocument();
  });

  // Spec 3.12: the count is in the trigger's accessible name, so a screen reader
  // hears what "+2" means, and its explanation is a Tooltip, not a native
  // `title` on a span no keyboard could reach.
  it('puts the count in the accessible name, and nothing in a native title', () => {
    state.primaryKb = base('alpha');
    state.visibleBases = [base('alpha'), base('beta'), base('gamma')];
    render(<KBSelectorTrigger onManage={() => {}} onCreate={() => {}} />);
    const trigger = screen.getByRole('button', { name: knowledgeCopy.selector.name('alpha', 2) });
    expect(trigger).toHaveAccessibleName('Knowledge base: alpha, and 2 more in this chat');
    expect(trigger.querySelectorAll('[title]')).toHaveLength(0);
  });

  // With no primary the name slot names nothing, so every visible base is
  // "other". Subtracting one unconditionally would undercount them.
  it('counts every visible base when there is no primary', () => {
    state.visibleBases = [base('alpha'), base('beta')];
    render(<KBSelectorTrigger onManage={() => {}} onCreate={() => {}} />);
    expect(screen.getByText('No primary knowledge base')).toBeInTheDocument();
    expect(screen.getByText('+2')).toBeInTheDocument();
  });

  it('shows no count when the primary is the only base in the chat', () => {
    state.primaryKb = base('alpha');
    state.visibleBases = [base('alpha')];
    render(<KBSelectorTrigger onManage={() => {}} onCreate={() => {}} />);
    expect(screen.queryByText(/^\+\d+$/)).not.toBeInTheDocument();
  });

  // The view that owns the dialogs the picker opens gives focus back to this button when one
  // closes (`useSelectorFocusReturn`), and can only do that if it can reach the element.
  it('hands its button to the view through triggerRef', () => {
    state.primaryKb = base('alpha');
    state.visibleBases = [base('alpha')];
    const triggerRef = createRef<HTMLButtonElement>();
    render(<KBSelectorTrigger onManage={() => {}} onCreate={() => {}} triggerRef={triggerRef} />);
    expect(triggerRef.current).toBe(screen.getByTestId('knowledge-kb-selector-trigger'));
  });
});
