import './compact-pane.css';
import { forwardRef, useId, useState } from 'react';
import type { SessionClassification } from '../../api';
import { usePrivacyTiersEnabled } from '../ConfigContext';
import { ChatKindIcon } from '../chats/ChatKindIcon';
import { chatIconFor, chatKindOf, chatPrivacyMark, type ChatKindSource } from '../chats/chatKind';
import { DICTATION_LABELS } from '../dictation/copy';
import { Maximize2 } from '../icons/app-icons';
import type { CompactPaneDensity, PaneShortfall } from '../Layout/yieldLadder';
import { Tooltip, TooltipContent, TooltipTrigger } from '../ui/Tooltip';
import { compactPaneCopy } from './copy';
import { useTabBandWindowGesture } from './useTabBandWindowGesture';

export type CompactPaneState = 'idle' | 'running' | 'waiting' | 'listening';

export interface CompactPaneViewProps {
  title: string;
  kindSource: ChatKindSource;
  tier?: SessionClassification | null;
  isActiveGroup: boolean;
  state: CompactPaneState;
  /** True while a turn is live (running or waiting): draws the dot. */
  live: boolean;
  shortfall: PaneShortfall;
  density: CompactPaneDensity;
  band: { insetStart: number; action: boolean; menu: React.ReactNode | null };
  action: { kind: 'grow' | 'merge'; label: string; run: () => void };
  announcement: string;
  animateIn: boolean;
}

/**
 * What a compact pane shows in place of its chat (rung 5 of the yield ladder).
 *
 * The pane's own 44px band (with the ▾ tab menu from two tabs) over a calm body
 * holding ONE quiet button: the chat's glyph in a 32px tile, its title, one
 * state line and the action words. One Tab stop, one hover target, one press.
 * The button is the same element in card and glyph density, so a density change
 * during a drag never moves focus; in band density the body is empty and the
 * action is a 24px button in the band. Its name and description never come
 * from the visible content, so nothing is lost to assistive tech in any density.
 *
 * Every rule is authored CSS in `compact-pane.css`; nothing here adds a
 * Tailwind utility (under BIOROUTER_NO_HMR a new one can silently fail to
 * generate). `sr-only` already exists.
 */
export const CompactPaneView = forwardRef<HTMLDivElement, CompactPaneViewProps>(
  function CompactPaneView(
    {
      title,
      kindSource,
      tier,
      isActiveGroup,
      state,
      live,
      shortfall,
      density,
      band,
      action,
      announcement,
      animateIn,
    },
    ref
  ) {
    const descriptionId = useId();
    const bandGesture = useTabBandWindowGesture();
    // "Mounting is not opening": the fade plays only when asked, once.
    const [entering, setEntering] = useState(animateIn);
    const tiersEnabled = usePrivacyTiersEnabled();
    const kindLabel = chatIconFor(
      chatKindOf(kindSource),
      chatPrivacyMark(tier, tiersEnabled)
    ).label;

    const stateText =
      state === 'listening'
        ? DICTATION_LABELS.listening
        : state === 'waiting'
          ? compactPaneCopy.waiting
          : state === 'running'
            ? compactPaneCopy.running
            : compactPaneCopy.shortfall[shortfall];
    const help = compactPaneCopy.help(shortfall, action.kind === 'grow');
    const actionName = compactPaneCopy.actionName(action.label, title);

    // It says what the person cannot fully see: the card shows the state line.
    const tooltip = (
      <TooltipContent side="bottom">
        <span className="block">{title}</span>
        {density !== 'card' ? <span className="block">{stateText}</span> : null}
        <span className="block">{help}</span>
      </TooltipContent>
    );

    return (
      <div
        ref={ref}
        // Out of the Tab order, and focusable: it holds focus when the band has
        // no room for the action, so focus never falls to the page. Its focus
        // edge is the shared focus-region one (main.css), repeated above the
        // band in compact-pane.css.
        className="br-compact-pane biorouter-focus-region"
        tabIndex={-1}
        role="group"
        aria-label={title}
        // With no action to carry it, the description is the group's own, so
        // the person who lands here hears the state and how the chat comes back.
        aria-describedby={density === 'band' && !band.action ? descriptionId : undefined}
        data-density={density}
        data-motion-layout=""
        data-enter={entering ? '' : undefined}
        onAnimationEnd={(event) => {
          if (event.target === event.currentTarget) setEntering(false);
        }}
      >
        <div
          className="br-compact-pane__band"
          // The empty band moves the window and zooms it on a double-click, as
          // the strip's does. By pointer gesture, never a `drag` app-region, so
          // it can never cover the floating titlebar controls (#74).
          onPointerDown={bandGesture.onPointerDown}
          onPointerUp={bandGesture.onPointerUp}
          onPointerCancel={bandGesture.onPointerCancel}
          onLostPointerCapture={bandGesture.onLostPointerCapture}
          onDoubleClick={bandGesture.onDoubleClick}
        >
          {density === 'band' && band.action ? (
            <Tooltip>
              <TooltipTrigger asChild>
                <button
                  type="button"
                  className="br-tabstrip__overflow biorouter-focus-surface"
                  data-compact-pane-action=""
                  data-testid="compact-pane-band-action"
                  aria-label={actionName}
                  aria-describedby={descriptionId}
                  onClick={action.run}
                >
                  <Maximize2 aria-hidden="true" />
                </button>
              </TooltipTrigger>
              {tooltip}
            </Tooltip>
          ) : null}
          {band.menu}
        </div>
        <div className="br-compact-pane__body">
          {density !== 'band' ? (
            <Tooltip>
              <TooltipTrigger asChild>
                <button
                  type="button"
                  className="br-compact-pane__action biorouter-focus-surface"
                  data-compact-pane-action=""
                  data-testid="compact-pane-action"
                  aria-label={actionName}
                  aria-describedby={descriptionId}
                  onClick={action.run}
                >
                  <span className="br-compact-pane__tile">
                    <ChatKindIcon session={kindSource} tier={tier} isActive={isActiveGroup} />
                    {live ? (
                      <span className="br-tab__dot br-compact-pane__dot" aria-hidden="true" />
                    ) : null}
                  </span>
                  <span className="br-compact-pane__title">{title}</span>
                  <span className="br-compact-pane__state" data-state={state}>
                    {stateText}
                  </span>
                  <span className="br-compact-pane__verb">
                    <Maximize2 aria-hidden="true" />
                    {action.label}
                  </span>
                </button>
              </TooltipTrigger>
              {tooltip}
            </Tooltip>
          ) : null}
        </div>
        <span id={descriptionId} className="sr-only">
          {`${kindLabel}. ${stateText}. ${help}`}
        </span>
        <span className="sr-only" role="status" aria-live="polite">
          {announcement}
        </span>
      </div>
    );
  }
);
