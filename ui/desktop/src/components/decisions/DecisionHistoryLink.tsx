import type { ReactNode } from 'react';
import { Button } from '../ui/button';
import { Tooltip, TooltipContent, TooltipTrigger } from '../ui/Tooltip';
import './decisionsPanel.css';

export const DECISION_HISTORY_LINK_COPY = {
  /** The visible words of every plain history link. */
  text: 'Decision history',
} as const;

export interface DecisionHistoryLinkProps {
  /**
   * What the link opens, in words: its accessible name and its tooltip ("Open this
   * decide call in the decision history"). It contains the visible words, so
   * label-in-name holds.
   */
  label: string;
  onOpen: () => void;
  /**
   * The quiet line form (a turn marker's source line): the children are the visible
   * text, in default ink, accent on hover and focus. Without children the visible
   * text is always "Decision history".
   */
  children?: ReactNode;
  testId?: string;
}

/**
 * One link to the decision history, one look, everywhere (F-13).
 *
 * - `Button variant="link" size="xs"` with `.br-decision-link` (decisionsPanel.css):
 *   accent ink, no underline at rest, underline on hover, never centred, no fill at any
 *   state, so it never puts a wash on the transcript well.
 * - Its label `Tooltip` (the 500ms rung) says what it opens, so a turn's link can be
 *   told from a card's before it is pressed.
 * - Placement: the plain form is the LAST line of its block, start-aligned, on a line of
 *   its own. Inside a dialog footer or a menu the same destination is a secondary
 *   button or a menu item with the words "Decision history", not this component.
 */
export function DecisionHistoryLink({ label, onOpen, children, testId }: DecisionHistoryLinkProps) {
  const line = children !== undefined && children !== null;
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button
          type="button"
          variant="link"
          size="xs"
          className={line ? 'br-decision-link br-decision-link--line' : 'br-decision-link'}
          aria-label={label}
          data-testid={testId}
          onClick={onOpen}
        >
          {line ? children : DECISION_HISTORY_LINK_COPY.text}
        </Button>
      </TooltipTrigger>
      <TooltipContent>{label}</TooltipContent>
    </Tooltip>
  );
}
