import * as React from 'react';
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { ToolsChip } from './ToolsChip';
import { TOOLS_COPY } from './copy';

/**
 * Spec 3.7: ONE Tools chip ("Tools" and a sum) opening ONE popover, with a
 * segmented control over the three lists. The lists themselves are tested in
 * their own files (`BottomMenu*Selection*.test.tsx`); here they are stand-ins
 * that record what the chip asks of them.
 */
const hooks = vi.hoisted(() => ({
  extensionsOpen: [] as boolean[],
  skillsOpen: [] as boolean[],
  knowledgeOpen: [] as boolean[],
  requestOpen: null as null | (() => void),
  setExtensionsQuery: vi.fn(),
  setSkillsQuery: vi.fn(),
  setKnowledgeQuery: vi.fn(),
}));

vi.mock('./BottomMenuExtensionSelection', () => ({
  useExtensionPicker: (
    _sessionId: string | null,
    { open, onOpenRequest }: { open: boolean; onOpenRequest?: () => void }
  ) => {
    hooks.extensionsOpen.push(open);
    hooks.requestOpen = onOpenRequest ?? null;
    return { activeCount: 4, setQuery: hooks.setExtensionsQuery };
  },
  ExtensionsPanel: ({ inputRef }: { inputRef?: React.Ref<HTMLInputElement> }) => (
    <input data-testid="extensions-panel" ref={inputRef} aria-label="extensions search" />
  ),
}));
vi.mock('./BottomMenuSkillSelection', () => ({
  useSkillPicker: (_sessionId: string | null, { open }: { open: boolean }) => {
    hooks.skillsOpen.push(open);
    return { activeCount: 9, setQuery: hooks.setSkillsQuery };
  },
  SkillsPanel: ({ inputRef }: { inputRef?: React.Ref<HTMLInputElement> }) => (
    <input data-testid="skills-panel" ref={inputRef} aria-label="skills search" />
  ),
}));
vi.mock('./BottomMenuKnowledgeSelection', () => ({
  useKnowledgePicker: ({ open }: { open: boolean }) => {
    hooks.knowledgeOpen.push(open);
    return { visibleCount: 2, setQuery: hooks.setKnowledgeQuery };
  },
  KnowledgePanel: ({
    inChat,
    inputRef,
  }: {
    inChat: boolean;
    inputRef?: React.Ref<HTMLInputElement>;
  }) => (
    <input
      data-testid="knowledge-panel"
      data-in-chat={String(inChat)}
      ref={inputRef}
      aria-label="knowledge search"
    />
  ),
}));

beforeAll(() => {
  vi.stubGlobal(
    'ResizeObserver',
    class {
      observe() {}
      unobserve() {}
      disconnect() {}
    }
  );
});
afterAll(() => vi.unstubAllGlobals());

beforeEach(() => {
  hooks.extensionsOpen = [];
  hooks.skillsOpen = [];
  hooks.knowledgeOpen = [];
  hooks.requestOpen = null;
  vi.clearAllMocks();
});

const last = (values: boolean[]) => values[values.length - 1];

describe('ToolsChip', () => {
  it('is one chip labelled Tools with the sum, and says the parts in its name', async () => {
    const user = userEvent.setup();
    render(<ToolsChip sessionId="chat-1" />);

    const chip = screen.getByTestId('composer-tools-chip');
    expect(chip).toHaveTextContent('Tools15');
    expect(chip).toHaveAccessibleName('Tools: 4 extensions, 9 skills, 2 knowledge bases');
    expect(chip).toHaveClass('br-picker-chip');
    expect(chip).not.toHaveAttribute('title');

    await user.hover(chip);
    expect(await screen.findByRole('tooltip')).toHaveTextContent(TOOLS_COPY.chipTooltip(4, 9, 2));
  });

  it('opens one popover with a segmented control over one list at a time', async () => {
    render(<ToolsChip sessionId="chat-1" />);
    const chip = screen.getByTestId('composer-tools-chip');
    expect(chip).toHaveAttribute('aria-expanded', 'false');
    fireEvent.click(chip);

    // `pickers.css` holds the chip's open ground and turns its chevron on
    // `aria-expanded`: the tooltip around the trigger overwrites `data-state`.
    expect(chip).toHaveAttribute('aria-expanded', 'true');
    const tabs = await screen.findByRole('radiogroup', { name: TOOLS_COPY.segmentsLabel });
    const radios = Array.from(tabs.querySelectorAll('[role="radio"]'));
    expect(radios.map((radio) => radio.textContent)).toEqual(['Extensions', 'Skills', 'Knowledge']);
    expect(screen.getByRole('radio', { name: 'Extensions' })).toHaveAttribute(
      'aria-checked',
      'true'
    );
    expect(screen.getByTestId('extensions-panel')).toBeInTheDocument();
    expect(screen.queryByTestId('skills-panel')).toBeNull();

    fireEvent.click(screen.getByRole('radio', { name: 'Skills' }));
    expect(await screen.findByTestId('skills-panel')).toBeInTheDocument();
    expect(screen.queryByTestId('extensions-panel')).toBeNull();

    fireEvent.click(screen.getByRole('radio', { name: 'Knowledge' }));
    expect(await screen.findByTestId('knowledge-panel')).toHaveAttribute('data-in-chat', 'true');
  });

  it('puts the segmented control last, under the list, where Tab reaches it after the list (R-25)', async () => {
    const user = userEvent.setup();
    render(<ToolsChip sessionId="chat-1" />);
    await user.click(screen.getByTestId('composer-tools-chip'));

    // The control is the popover's last child: it sits on the bottom edge, which
    // is the edge pinned above the chip (geometry: toolsPopover.browser.test.tsx).
    const popover = await screen.findByTestId('composer-tools-popover');
    const tabs = screen.getByTestId('composer-tools-tabs');
    expect(popover.lastElementChild).toBe(tabs);
    const search = screen.getByTestId('extensions-panel');
    expect(search.compareDocumentPosition(tabs) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();

    // The DOM order is the visual order: the search first, the control after it.
    await waitFor(() => expect(search).toHaveFocus());
    await user.tab();
    expect(screen.getByRole('radio', { name: 'Extensions' })).toHaveFocus();

    // One Tab stop; the arrow keys switch lists, and the new list comes up above.
    // Held, as a person's key is: Radix selects on the focus an arrow key moves.
    await user.keyboard('{ArrowRight>}');
    await waitFor(() =>
      expect(screen.getByRole('radio', { name: 'Skills' })).toHaveAttribute('aria-checked', 'true')
    );
    await user.keyboard('{/ArrowRight}');
    expect(await screen.findByTestId('skills-panel')).toBeInTheDocument();
    expect(popover.lastElementChild).toBe(tabs);
  });

  it('puts focus in the search field when it opens', async () => {
    render(<ToolsChip sessionId="chat-1" />);
    fireEvent.click(screen.getByTestId('composer-tools-chip'));
    await waitFor(() => expect(screen.getByTestId('extensions-panel')).toHaveFocus());
  });

  it('tells only the list on screen that it is open, so each re-reads when shown', async () => {
    render(<ToolsChip sessionId="chat-1" />);
    expect(last(hooks.extensionsOpen)).toBe(false);

    fireEvent.click(screen.getByTestId('composer-tools-chip'));
    await screen.findByTestId('extensions-panel');
    expect(last(hooks.extensionsOpen)).toBe(true);
    expect(last(hooks.skillsOpen)).toBe(false);
    expect(last(hooks.knowledgeOpen)).toBe(false);

    fireEvent.click(screen.getByRole('radio', { name: 'Skills' }));
    await screen.findByTestId('skills-panel');
    expect(last(hooks.extensionsOpen)).toBe(false);
    expect(last(hooks.skillsOpen)).toBe(true);
  });

  it('opens on Extensions when the app asks to show this chat’s extensions', async () => {
    render(<ToolsChip sessionId="chat-1" />);
    fireEvent.click(screen.getByTestId('composer-tools-chip'));
    fireEvent.click(await screen.findByRole('radio', { name: 'Knowledge' }));
    fireEvent.keyDown(screen.getByTestId('knowledge-panel'), { key: 'Escape' });
    await waitFor(() => expect(screen.queryByTestId('knowledge-panel')).toBeNull());

    act(() => hooks.requestOpen?.());
    expect(await screen.findByTestId('extensions-panel')).toBeInTheDocument();
  });

  it('clears every search when it closes', async () => {
    render(<ToolsChip sessionId="chat-1" />);
    fireEvent.click(screen.getByTestId('composer-tools-chip'));
    await screen.findByTestId('extensions-panel');
    fireEvent.keyDown(screen.getByTestId('extensions-panel'), { key: 'Escape' });
    await waitFor(() => expect(screen.queryByTestId('extensions-panel')).toBeNull());
    expect(hooks.setExtensionsQuery).toHaveBeenCalledWith('');
    expect(hooks.setSkillsQuery).toHaveBeenCalledWith('');
    expect(hooks.setKnowledgeQuery).toHaveBeenCalledWith('');
  });

  it('gives focus back to the chip after Escape', async () => {
    render(<ToolsChip sessionId="chat-1" />);
    const chip = screen.getByTestId('composer-tools-chip');
    fireEvent.click(chip);
    await screen.findByTestId('extensions-panel');
    fireEvent.keyDown(screen.getByTestId('extensions-panel'), { key: 'Escape' });
    await waitFor(() => expect(chip).toHaveFocus());
  });

  it('lists Knowledge as the default chats inherit on Home', async () => {
    render(<ToolsChip sessionId={null} />);
    fireEvent.click(screen.getByTestId('composer-tools-chip'));
    fireEvent.click(await screen.findByRole('radio', { name: 'Knowledge' }));
    expect(await screen.findByTestId('knowledge-panel')).toHaveAttribute('data-in-chat', 'false');
  });
});
