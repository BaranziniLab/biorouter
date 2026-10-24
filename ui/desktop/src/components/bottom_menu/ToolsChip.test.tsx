import * as React from 'react';
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { ToolsChip } from './ToolsChip';
import { TOOLS_COPY } from './copy';
import { TOOLTIP_DELAY_MS } from '../ui/Tooltip';

/**
 * ONE Tools chip with its counts on hover opening ONE popover, with a
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
  // What each list reports: its count, and whether it has answered for the chat.
  extensions: { count: 4, answered: true },
  skills: { count: 9, answered: true },
  knowledge: { count: 2, answered: true },
}));

vi.mock('./BottomMenuExtensionSelection', () => ({
  useExtensionPicker: (
    _sessionId: string | null,
    { open, onOpenRequest }: { open: boolean; onOpenRequest?: () => void }
  ) => {
    hooks.extensionsOpen.push(open);
    hooks.requestOpen = onOpenRequest ?? null;
    return {
      activeCount: hooks.extensions.count,
      answered: hooks.extensions.answered,
      setQuery: hooks.setExtensionsQuery,
    };
  },
  ExtensionsPanel: ({ inputRef }: { inputRef?: React.Ref<HTMLInputElement> }) => (
    <input data-testid="extensions-panel" ref={inputRef} aria-label="extensions search" />
  ),
}));
vi.mock('./BottomMenuSkillSelection', () => ({
  useSkillPicker: (_sessionId: string | null, { open }: { open: boolean }) => {
    hooks.skillsOpen.push(open);
    return {
      activeCount: hooks.skills.count,
      answered: hooks.skills.answered,
      setQuery: hooks.setSkillsQuery,
    };
  },
  SkillsPanel: ({ inputRef }: { inputRef?: React.Ref<HTMLInputElement> }) => (
    <input data-testid="skills-panel" ref={inputRef} aria-label="skills search" />
  ),
}));
vi.mock('./BottomMenuKnowledgeSelection', () => ({
  useKnowledgePicker: ({ open }: { open: boolean }) => {
    hooks.knowledgeOpen.push(open);
    return {
      visibleCount: hooks.knowledge.count,
      answered: hooks.knowledge.answered,
      setQuery: hooks.setKnowledgeQuery,
    };
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
  hooks.extensions = { count: 4, answered: true };
  hooks.skills = { count: 9, answered: true };
  hooks.knowledge = { count: 2, answered: true };
  vi.clearAllMocks();
});

const last = (values: boolean[]) => values[values.length - 1];
const toolCounts = (chip: HTMLElement) => chip.getAttribute('aria-label')?.match(/\d+/g) ?? [];

describe('ToolsChip', () => {
  it('shows only Tools, and gives the enabled total and category counts on hover', async () => {
    const user = userEvent.setup();
    render(<ToolsChip sessionId="chat-1" />);

    const chip = screen.getByTestId('composer-tools-chip');
    expect(chip).toHaveTextContent(/^Tools$/);
    expect(toolCounts(chip)).toEqual(['4', '9', '2']);
    expect(chip).toHaveAccessibleName('Tools: 4 extensions, 9 skills, 2 knowledge bases');
    expect(chip).toHaveClass('br-picker-chip');
    expect(chip).not.toHaveAttribute('title');
    expect(chip.querySelectorAll('svg')).toHaveLength(1);

    await user.hover(chip);
    const tooltip = await screen.findByRole('tooltip');
    expect(tooltip).toHaveTextContent(/15 (?:enabled tools|tools enabled)/);
    expect(tooltip).toHaveTextContent('4 extensions');
    expect(tooltip).toHaveTextContent('9 skills');
    expect(tooltip).toHaveTextContent('2 knowledge bases');
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

  it('hands focus to the new list’s search after a pointer pick, and keeps it on the control after an arrow key', async () => {
    const user = userEvent.setup();
    render(<ToolsChip sessionId="chat-1" />);
    await user.click(screen.getByTestId('composer-tools-chip'));
    await waitFor(() => expect(screen.getByTestId('extensions-panel')).toHaveFocus());

    // Pointer: "click Skills, type gg" filters the skills.
    await user.click(screen.getByRole('radio', { name: 'Skills' }));
    await waitFor(() => expect(screen.getByTestId('skills-panel')).toHaveFocus());
    await user.keyboard('gg');
    expect(screen.getByTestId('skills-panel')).toHaveValue('gg');

    // Keyboard: Tab to the control, then an arrow switches the list and focus
    // stays on the control, so the next arrow keeps switching.
    await user.tab();
    expect(screen.getByRole('radio', { name: 'Skills' })).toHaveFocus();
    await user.keyboard('{ArrowRight>}');
    await waitFor(() =>
      expect(screen.getByRole('radio', { name: 'Knowledge' })).toHaveAttribute(
        'aria-checked',
        'true'
      )
    );
    await user.keyboard('{/ArrowRight}');
    expect(await screen.findByTestId('knowledge-panel')).toBeInTheDocument();
    expect(screen.getByRole('radio', { name: 'Knowledge' })).toHaveFocus();
    expect(screen.getByTestId('knowledge-panel')).not.toHaveFocus();
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

  describe('the counts state this chat’s tools, or remain unknown (V2-R7-3)', () => {
    /**
     * Every count tuple the chip has named, in order, from the first commit on. The
     * observer reports in a microtask, so each step is flushed before the next.
     */
    function recordNumbers(chip: HTMLElement): string[] {
      const seen: string[] = [];
      const note = () => {
        const shown = toolCounts(chip).join('|');
        if (seen[seen.length - 1] !== shown) seen.push(shown);
      };
      note();
      new MutationObserver(note).observe(chip, {
        attributes: true,
        attributeFilter: ['aria-label'],
      });
      return seen;
    }
    const flush = () => act(async () => {});

    it('keeps counts unknown until all three lists have answered, then names them once', async () => {
      const user = userEvent.setup();
      hooks.extensions = { count: 2, answered: false };
      hooks.skills = { count: 0, answered: false };
      hooks.knowledge = { count: 1, answered: false };
      const { rerender } = render(<ToolsChip sessionId="chat-1" />);
      const chip = screen.getByTestId('composer-tools-chip');
      const numbers = recordNumbers(chip);

      expect(toolCounts(chip)).toEqual([]);
      expect(chip).toHaveTextContent(/^Tools$/);
      expect(chip).toHaveAccessibleName(TOOLS_COPY.chip);
      // Nothing to say on hover either: the parts are not known. (Held past the
      // delay a ready chip's tooltip takes, so the check cannot pass early.)
      await user.hover(chip);
      await act(() => new Promise((resolve) => setTimeout(resolve, TOOLTIP_DELAY_MS + 200)));
      expect(screen.queryByRole('tooltip')).toBeNull();
      await user.unhover(chip);

      // The lists answer one at a time, as they did on a chat switch: the
      // configuration's fallback first, then the chat's own figures.
      hooks.knowledge = { count: 0, answered: true };
      rerender(<ToolsChip sessionId="chat-1" />);
      await flush();
      hooks.skills = { count: 3, answered: true };
      rerender(<ToolsChip sessionId="chat-1" />);
      await flush();
      expect(toolCounts(chip)).toEqual([]);
      expect(chip).toHaveTextContent(/^Tools$/);
      hooks.extensions = { count: 1, answered: true };
      rerender(<ToolsChip sessionId="chat-1" />);
      await flush();

      expect(toolCounts(chip)).toEqual(['1', '3', '0']);
      expect(chip).toHaveAccessibleName('Tools: 1 extension, 3 skills, 0 knowledge bases');
      // No partial figures on the way: unknown, then the settled tuple.
      expect(numbers).toEqual(['', '1|3|0']);
    });

    it('never shows the previous chat’s counts under the next one', async () => {
      const { rerender } = render(<ToolsChip sessionId="chat-a" />);
      const chip = screen.getByTestId('composer-tools-chip');
      expect(toolCounts(chip)).toEqual(['4', '9', '2']);
      const numbers = recordNumbers(chip);

      // The tab keeps its mount and binds another chat: the lists still hold
      // chat A's figures until chat B's answers land.
      hooks.extensions = { count: 4, answered: false };
      hooks.skills = { count: 9, answered: false };
      rerender(<ToolsChip sessionId="chat-b" />);
      await flush();
      expect(toolCounts(chip)).toEqual([]);

      hooks.extensions = { count: 1, answered: true };
      hooks.skills = { count: 2, answered: true };
      rerender(<ToolsChip sessionId="chat-b" />);
      await flush();
      expect(toolCounts(chip)).toEqual(['1', '2', '2']);
      expect(numbers).toEqual(['4|9|2', '', '1|2|2']);
    });

    it('keeps the counts while one list re-asks for the same chat, and moves them once', async () => {
      const { rerender } = render(<ToolsChip sessionId="chat-1" />);
      const chip = screen.getByTestId('composer-tools-chip');
      const numbers = recordNumbers(chip);

      // The knowledge selection follows the window's focused chat, which moves
      // between split panes: it asks again, and its cached guess is not shown.
      hooks.knowledge = { count: 5, answered: false };
      rerender(<ToolsChip sessionId="chat-1" />);
      await flush();
      expect(toolCounts(chip)).toEqual(['4', '9', '2']);

      hooks.knowledge = { count: 3, answered: true };
      rerender(<ToolsChip sessionId="chat-1" />);
      await flush();
      expect(toolCounts(chip)).toEqual(['4', '9', '3']);
      expect(numbers).toEqual(['4|9|2', '4|9|3']);
    });

    it('carries the counts into a chat this composer has just started', async () => {
      // A new tab's composer: no chat yet, counting what the next chat gets.
      const { rerender } = render(<ToolsChip sessionId={null} />);
      const chip = screen.getByTestId('composer-tools-chip');
      const numbers = recordNumbers(chip);

      // The first message creates the chat; its lists have not answered yet.
      hooks.extensions = { count: 4, answered: false };
      hooks.skills = { count: 9, answered: false };
      rerender(<ToolsChip sessionId="chat-new" />);
      await flush();
      expect(toolCounts(chip)).toEqual(['4', '9', '2']);

      hooks.extensions = { count: 4, answered: true };
      hooks.skills = { count: 9, answered: true };
      rerender(<ToolsChip sessionId="chat-new" />);
      await flush();
      expect(numbers).toEqual(['4|9|2']);
    });

    it('names carried counts as it names answered ones', async () => {
      const { rerender } = render(<ToolsChip sessionId="chat-1" />);
      const chip = screen.getByTestId('composer-tools-chip');
      hooks.knowledge = { count: 5, answered: false };
      rerender(<ToolsChip sessionId="chat-1" />);
      await flush();
      // The accessible counts remain settled while the list refreshes (V2-R8-3).
      expect(toolCounts(chip)).toEqual(['4', '9', '2']);
      expect(chip).toHaveAccessibleName('Tools: 4 extensions, 9 skills, 2 knowledge bases');
    });

    it('hands its counts to the chip that replaces it when the composer remounts (V2-R8-3)', async () => {
      // A new chat's first send moves the composer from the centre of the empty
      // chat to the bar under the transcript: a second mount, in one commit.
      function Seat({ centre, sessionId }: { centre: boolean; sessionId: string | null }) {
        const chip = <ToolsChip sessionId={sessionId} handOffKey="tab-1" />;
        return (
          <div>
            {centre ? <section>{chip}</section> : <p />}
            {!centre && <footer>{chip}</footer>}
          </div>
        );
      }
      const { rerender } = render(<Seat centre sessionId={null} />);
      const first = screen.getByTestId('composer-tools-chip');
      expect(toolCounts(first)).toEqual(['4', '9', '2']);

      // The new mount's lists have not answered yet, for "no chat" or the chat.
      hooks.extensions = { count: 4, answered: false };
      hooks.skills = { count: 9, answered: false };
      rerender(<Seat centre={false} sessionId={null} />);
      const second = screen.getByTestId('composer-tools-chip');
      expect(second).not.toBe(first);
      const numbers = recordNumbers(second);
      expect(second).toHaveAccessibleName('Tools: 4 extensions, 9 skills, 2 knowledge bases');

      rerender(<Seat centre={false} sessionId="chat-new" />);
      await flush();
      hooks.extensions = { count: 4, answered: true };
      hooks.skills = { count: 9, answered: true };
      rerender(<Seat centre={false} sessionId="chat-new" />);
      await flush();
      expect(numbers).toEqual(['4|9|2']);

      // Not for another chat: a remount onto a different chat starts from nothing.
      hooks.extensions = { count: 4, answered: false };
      rerender(<Seat centre sessionId="chat-other" />);
      expect(toolCounts(screen.getByTestId('composer-tools-chip'))).toEqual([]);
    });
  });
});
