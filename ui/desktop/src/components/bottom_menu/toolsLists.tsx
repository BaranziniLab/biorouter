import * as React from 'react';
import { useState } from 'react';
import { readComposerHandOff, useComposerHandOff } from '../composer/composerHandOff';
import {
  ExtensionsPanel,
  useExtensionPicker,
  type ExtensionPicker,
} from './BottomMenuExtensionSelection';
import {
  KnowledgePanel,
  useKnowledgePicker,
  type KnowledgePicker,
} from './BottomMenuKnowledgeSelection';
import { SkillsPanel, useSkillPicker, type SkillPicker } from './BottomMenuSkillSelection';

/** The three tool lists the `+` popover switches between. */
export type ToolsSegment = 'extensions' | 'skills' | 'knowledge';

/** Extensions, skills and knowledge bases on in the chat, in that order. */
export type ToolCounts = readonly [extensions: number, skills: number, knowledge: number];

/** The three lists' state, held for as long as the composer is mounted. */
export interface ToolsLists {
  extensions: ExtensionPicker;
  skills: SkillPicker;
  knowledge: KnowledgePicker;
  /**
   * The counts to state, or `null` while they are not known for THIS chat
   * (V2-R7-3): shown only once all three lists have answered.
   */
  shown: ToolCounts | null;
  /** Empty every list's search field (the popover closed). */
  clearQueries: () => void;
}

/** The three figures last stated, and the chat they were for. */
interface SettledTools {
  chat: string | null;
  counts: ToolCounts;
}

const TOOLS_HAND_OFF = 'tools-chip';

/**
 * The composer's three tool lists (Extensions, Skills, Knowledge): their state,
 * fetches and toggles, and the counts they add up to. `ComposerPlusMenu` holds
 * them for the whole time the composer is mounted, so the `+` states the right
 * counts before its popover is ever opened. Each list re-reads its source when
 * it comes on screen (`segment`).
 *
 * One module, so a composer test stubs the lists in one line
 * (`vi.mock('./bottom_menu/toolsLists', …)`) rather than three.
 *
 * ⚠ **Counts are exposed only once all three lists have answered for THIS chat**
 * (V2-R7-3, principle 10): summed as each arrived, a chat switch read 2, then 5,
 * then 4, a number shown and then withdrawn. Two cases keep the last settled
 * figures instead of hiding them: the same chat while one list asks again (the
 * knowledge selection follows the window's focused chat), and a chat THIS
 * composer has just started. That first message also REMOUNTS the composer
 * (V2-R8-3), so the settled figures are handed to the new mount under
 * `handOffKey` (`composer/composerHandOff.ts`).
 */
export function useToolsLists(
  sessionId: string | null,
  {
    open,
    segment,
    handOffKey,
    onOpenRequest,
  }: {
    /** Whether the `+` popover is open. */
    open: boolean;
    /** Which list it shows, or `null` for one that is not a tool list. */
    segment: ToolsSegment | null;
    handOffKey?: string;
    /** The app asked to show this chat's extensions. */
    onOpenRequest?: () => void;
  }
): ToolsLists {
  const extensions = useExtensionPicker(sessionId, {
    open: open && segment === 'extensions',
    onOpenRequest,
  });
  const skills = useSkillPicker(sessionId, { open: open && segment === 'skills' });
  const knowledge = useKnowledgePicker({ open: open && segment === 'knowledge' });

  const counts: ToolCounts = [
    extensions.activeCount,
    skills.activeCount,
    knowledge.visibleCount,
  ] as const;
  const chat = sessionId || null;
  const ready = extensions.answered && skills.answered && knowledge.answered;
  const [settled, setSettled] = useState<SettledTools | null>(() =>
    readComposerHandOff<SettledTools>(handOffKey, TOOLS_HAND_OFF)
  );
  if (
    ready &&
    (settled === null || settled.chat !== chat || settled.counts.some((n, i) => n !== counts[i]))
  ) {
    setSettled({ chat, counts });
  }
  useComposerHandOff(handOffKey, TOOLS_HAND_OFF, settled);
  const shown: ToolCounts | null = ready
    ? counts
    : settled !== null && (settled.chat === chat || (settled.chat === null && chat !== null))
      ? settled.counts
      : null;

  return {
    extensions,
    skills,
    knowledge,
    shown,
    clearQueries: () => {
      extensions.setQuery('');
      skills.setQuery('');
      knowledge.setQuery('');
    },
  };
}

/** The list a segment shows, each handed the popover's focus target. */
export function ToolsListPanel({
  lists,
  segment,
  inChat,
  inputRef,
}: {
  lists: ToolsLists;
  segment: ToolsSegment;
  /** Whether the composer is in a chat (Home's knowledge list shapes new chats). */
  inChat: boolean;
  inputRef?: React.Ref<HTMLInputElement>;
}) {
  if (segment === 'extensions')
    return <ExtensionsPanel picker={lists.extensions} inputRef={inputRef} />;
  if (segment === 'skills') return <SkillsPanel picker={lists.skills} inputRef={inputRef} />;
  return <KnowledgePanel picker={lists.knowledge} inChat={inChat} inputRef={inputRef} />;
}
