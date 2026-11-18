import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterAll, beforeAll, describe, expect, it, vi } from 'vitest';
import { ComposerPlusMenu } from './ComposerPlusMenu';
import { TOOLS_COPY } from '../bottom_menu/copy';
import type { KnowledgePicker } from '../bottom_menu/BottomMenuKnowledgeSelection';

/**
 * rq2-107 (K-23). A draft "New chat" tab hands its composer the session id `''`:
 * there is no chat yet, and its Knowledge list edits the default new chats
 * inherit, exactly as Home's does. The scope line said "This chat" there, because
 * the `+` asked `sessionId !== null` and `''` is not `null`.
 *
 * The real Knowledge list is rendered, so the words checked are the ones a person
 * reads; only the three lists' state is a stand-in.
 */
vi.mock('../bottom_menu/BottomMenuExtensionSelection', () => ({
  useExtensionPicker: () => ({ activeCount: 1, answered: true, setQuery: vi.fn() }),
  ExtensionsPanel: () => null,
}));
vi.mock('../bottom_menu/BottomMenuSkillSelection', () => ({
  useSkillPicker: () => ({ activeCount: 1, answered: true, setQuery: vi.fn() }),
  SkillsPanel: () => null,
}));
vi.mock('../bottom_menu/BottomMenuKnowledgeSelection', async (importOriginal) => {
  const actual =
    await importOriginal<typeof import('../bottom_menu/BottomMenuKnowledgeSelection')>();
  const picker: KnowledgePicker = {
    visibleCount: 1,
    answered: true,
    query: '',
    setQuery: () => {},
    bases: [],
    isHidden: () => false,
    toggle: () => {},
    bulk: null,
  };
  return { ...actual, useKnowledgePicker: () => picker };
});

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

async function knowledgeListFor(sessionId: string | null) {
  const user = userEvent.setup();
  const view = render(
    <ComposerPlusMenu sessionId={sessionId} onAttachFiles={vi.fn()} onInsertTrigger={vi.fn()} />
  );
  await user.click(screen.getByTestId('composer-plus'));
  await user.click(screen.getByRole('radio', { name: TOOLS_COPY.segments.knowledge }));
  return { panel: await screen.findByTestId('tools-knowledge-panel'), unmount: view.unmount };
}

describe('the `+` popover’s Knowledge list in a chat that has not started', () => {
  it('a_draft_tab_says_default_for_chats', async () => {
    const { panel } = await knowledgeListFor('');
    expect(within(panel).getByText(TOOLS_COPY.scope.defaultForChats)).toBeInTheDocument();
    expect(within(panel).queryByText(TOOLS_COPY.scope.thisChat)).toBeNull();
  });

  it('says the same on Home, and "This chat" in a chat that exists', async () => {
    const home = await knowledgeListFor(null);
    expect(within(home.panel).getByText(TOOLS_COPY.scope.defaultForChats)).toBeInTheDocument();
    home.unmount();

    const chat = await knowledgeListFor('20261116_1');
    expect(within(chat.panel).getByText(TOOLS_COPY.scope.thisChat)).toBeInTheDocument();
    expect(within(chat.panel).queryByText(TOOLS_COPY.scope.defaultForChats)).toBeNull();
  });
});
