import type { SessionClassification } from '../../api';
import type { ChatKindSource } from '../chats/chatKind';
import type { PaneShortfall } from '../Layout/yieldLadder';
import type { ChatGroupId, ChatTab, ChatTabId, GroupLayout } from './chatGroupsTypes';
import type { PaneResize } from './paneGrowPlan';

/** What a pane's chat is, for its placeholder: the same inputs its tab uses. */
export interface PaneIdentity {
  title: string;
  kindSource: ChatKindSource;
  /** Absent means not yet known; never drawn as Public. */
  tier?: SessionClassification | null;
  sessionId: string;
}

/** Everything the shell hands a pane so it can draw and act as a compact pane. */
export interface CompactPaneInputs {
  identity: PaneIdentity;
  /** For the band's ▾: the same inputs the strip's overflow menu takes. */
  tabMenu: {
    tabs: ChatTab[];
    activeTabId: ChatTabId | null;
    runningSessionIds: readonly string[];
    onSelectTab: (tabId: ChatTabId) => void;
    renderGlyph: (tab: ChatTab) => React.ReactNode;
    titleOf: (tab: ChatTab) => string;
  };
  layout: GroupLayout;
  treeRef: React.RefObject<HTMLDivElement | null>;
  /** 172 for the first leaf while reserveTitlebarControls is true, else 0. */
  bandInsetStart: number;
  onGrow: (plan: PaneResize[]) => void;
  onMerge: () => void;
}

export interface CompactPaneProps extends CompactPaneInputs {
  groupId: ChatGroupId;
  tabKey: string;
  isActiveGroup: boolean;
  shortfall: PaneShortfall;
  animateIn: boolean;
}

/** The placeholder a compact pane shows in place of its chat (rung 5). */
export function CompactPane({ identity }: CompactPaneProps) {
  return (
    <div className="br-compact-pane" role="group" aria-label={identity.title}>
      <button type="button" data-compact-pane-action="" data-testid="compact-pane-action">
        {identity.title}
      </button>
    </div>
  );
}
