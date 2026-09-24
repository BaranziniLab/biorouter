import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type FocusEvent,
  type KeyboardEvent,
  type ReactNode,
} from 'react';
import { ArrowLeft, X } from '../../icons/app-icons';
import { Button } from '../../ui/button';
import { Tabs, TabsContent, TabsList, TabsTrigger } from '../../ui/tabs';
import { Tooltip, TooltipContent, TooltipTrigger } from '../../ui/Tooltip';
import { cn } from '../../../utils';
import { channelName } from '../identity';
import type { DetailsTab, PaneIntent, PaneMode as PaneModeName } from '../state/types';
import { AboutTab } from './AboutTab';
import { AgentTaskPane } from './AgentTaskPane';
import { agentCopy, paneCopy } from './copy';
import { MembersTab } from './MembersTab';
import { usePanePresentation } from './presentation';
import './pane.css';

const TABS: readonly DetailsTab[] = ['about', 'members', 'files', 'access'];

/** The pinned composer label every channel's composer carries: the fallback focus target. */
const COMPOSER_SELECTOR = 'textarea[aria-label^="Message #"]';

export interface DetailsPaneProps {
  /**
   * The details tabs. About and Members default to this area's own; Files and Access come from
   * the files and access areas.
   */
  tabs?: Partial<Record<DetailsTab, ReactNode>>;
  /**
   * Ask my agent. Defaults to this area's `AgentTaskPane`, which has no way to reach the timeline.
   * The layout passes `<AgentTaskPane onShowTask={…} />` so "Show task in channel" highlights the
   * task's row.
   */
  agent?: ReactNode;
  /** Chat access (grant and revoke), from the access area. */
  chatAccess?: ReactNode;
  /** Layout only, on the `<aside>`. */
  className?: string;
}

/** The control that opened the pane: a menu item stands for the trigger of its menu. */
function openerOf(element: Element | null, pane: HTMLElement | null): HTMLElement | null {
  if (!(element instanceof HTMLElement) || element === document.body) return null;
  if (pane?.contains(element)) return null;
  const menu = element.closest('[role="menu"]');
  if (menu) {
    const trigger = document.getElementById(menu.getAttribute('aria-labelledby') ?? '');
    return trigger instanceof HTMLElement ? trigger : null;
  }
  return element;
}

function focusable(element: HTMLElement | null): element is HTMLElement {
  return (
    element !== null &&
    element.isConnected &&
    !element.hasAttribute('disabled') &&
    element.getAttribute('aria-hidden') !== 'true' &&
    element.closest('[inert]') === null
  );
}

function titleOf(intent: PaneIntent, channel: string): string {
  switch (intent.mode) {
    case 'details':
      return channel;
    case 'agent':
      return agentCopy.title;
    case 'chat-access':
      return paneCopy.chatAccessTitle;
  }
}

function landmarkOf(intent: PaneIntent, channel: string): string {
  return intent.mode === 'details' ? paneCopy.detailsName(channel) : titleOf(intent, channel);
}

/** The ×'s name: what it closes (Q2-68). */
function closeNameOf(intent: PaneIntent): string {
  switch (intent.mode) {
    case 'details':
      return paneCopy.close;
    case 'agent':
      return paneCopy.closeAgent;
    case 'chat-access':
      return paneCopy.closeChatAccess;
  }
}

/** One mode's content. Keyed by mode, so a mode change mounts it afresh (and animates it in). */
function PaneMode({ animate, children }: { animate: boolean; children: ReactNode }) {
  // Decided once, when this mode mounts: a later re-render must not cut the entrance short.
  const [entrance] = useState(animate);
  return (
    <div className={cn('crew-pane-mode', entrance && 'animate-fade-slide-up')}>{children}</div>
  );
}

/**
 * The one details pane (ui-redesign-spec, "The details pane"): a non-modal `<aside>` beside the
 * conversation — no scrim, no focus trap, nothing outside it `aria-hidden` — with one mode at a
 * time: channel details (About · Members · Files · Access), Ask my agent, or Chat access.
 *
 * It renders the `.crew-pane` grid item itself (place it directly inside `.crew-stage`), and stays
 * mounted across modes, so switching modes replaces the content in place. It follows the
 * controller's pane intent and never closes itself on a refresh: the controller closes it on an
 * explicit close, a channel or connection switch, or lost access. `Escape` closes it while focus
 * is inside, and closing returns focus to the control that opened it (else the composer) — but
 * only when focus was in the pane, so a click elsewhere keeps its focus.
 *
 * In cover mode (a narrow window; a container query in `crew-app.css`) the header gains
 * "← Back to #name", and the details mode's title becomes "Details" so the channel is not named
 * twice in one row (T-46).
 *
 * Leaving Ask my agent — closing the pane or switching its mode — dismisses an error that mode
 * reported (`pane:agent`), so it does not fall back to the connection bar for a drawer that is
 * gone (T-48). One reported after the pane left is news, and still reaches the bar.
 */
export function DetailsPane({ tabs = {}, agent, chatAccess, className }: DetailsPaneProps) {
  const { crew, channel } = usePanePresentation();
  const intent = crew.ui.pane;
  const open = intent !== null;
  const aside = useRef<HTMLElement>(null);
  const heading = useRef<HTMLHeadingElement>(null);
  const opener = useRef<HTMLElement | null>(null);
  const focusInside = useRef(false);
  const previousIntent = useRef<PaneIntent | null>(null);
  const [leaving, setLeaving] = useState<PaneIntent | null>(null);
  const shown = intent ?? leaving;
  const title = channel ? channelName(channel) : '';
  const tab: DetailsTab = intent?.mode === 'details' ? (intent.tab ?? 'about') : 'about';
  const focusKey = intent ? `${intent.mode}:${intent.mode === 'details' ? tab : ''}` : null;

  // Open: remember the opener, and again when a control outside the pane switches its mode
  // (Members → Ask my agent from the composer), so Escape returns to that control rather than to
  // the one that first opened it. A switch made from inside the pane keeps the opener. Close:
  // keep the last content while an exit animation runs (a pushed pane narrows over it); with no
  // animation (reduced motion, or no stylesheet) it goes at once.
  useLayoutEffect(() => {
    const was = previousIntent.current;
    previousIntent.current = intent;
    if (!was && intent) {
      opener.current = openerOf(document.activeElement, aside.current);
      setLeaving(null);
    } else if (was && intent && was.mode !== intent.mode) {
      const next = openerOf(document.activeElement, aside.current);
      if (next) opener.current = next;
    } else if (was && !intent) {
      const name = aside.current ? getComputedStyle(aside.current).animationName : '';
      if (name && name !== 'none') setLeaving(was);
    }
  }, [intent]);

  // T-48: leaving Ask my agent takes its error with it.
  const { error, dismissError } = crew;
  const previousMode = useRef<PaneModeName | null>(null);
  useEffect(() => {
    const was = previousMode.current;
    const now = intent?.mode ?? null;
    previousMode.current = now;
    if (was === 'agent' && now !== 'agent' && error?.source === 'pane:agent') dismissError();
  }, [intent?.mode, error, dismissError]);

  useEffect(() => {
    if (!leaving) return;
    const fallback = window.setTimeout(() => setLeaving(null), 600);
    return () => window.clearTimeout(fallback);
  }, [leaving]);

  // Move focus into the pane when it opens or changes what it shows.
  useEffect(() => {
    if (!focusKey) return;
    const pane = aside.current;
    if (!pane) return;
    const mode = focusKey.split(':')[0];
    const target =
      mode === 'details'
        ? pane.querySelector<HTMLElement>('[role="tab"][data-state="active"]')
        : mode === 'agent'
          ? pane.querySelector<HTMLElement>('[data-crew-pane-autofocus]')
          : null;
    (target ?? heading.current)?.focus();
  }, [focusKey]);

  // Closed: hand focus back if it was in the pane (or was lost with the pane's content).
  const wasOpen = useRef(false);
  useEffect(() => {
    if (open) {
      wasOpen.current = true;
      return;
    }
    if (!wasOpen.current) return;
    wasOpen.current = false;
    const wasInside = focusInside.current || document.activeElement === document.body;
    focusInside.current = false;
    const back = opener.current;
    opener.current = null;
    if (!wasInside) return;
    if (focusable(back)) back.focus();
    else document.querySelector<HTMLElement>(COMPOSER_SELECTOR)?.focus();
  }, [open]);

  const onKeyDown = (event: KeyboardEvent<HTMLElement>) => {
    // Portalled content (a popover, a menu) bubbles here through React, not through the DOM; it
    // handles its own Escape.
    if (event.key !== 'Escape' || event.defaultPrevented) return;
    if (!aside.current?.contains(event.target as Node)) return;
    event.preventDefault();
    crew.closePane();
  };
  // The ×'s own Escape (Q2-68). Focused by keyboard, it shows its tooltip, and the tooltip's
  // dismissable layer takes Escape first — at the document, in the capture phase — and marks it
  // handled, so the pane's handler above let it go and Escape on "Close details" did nothing.
  // Escape on the control whose whole job is closing the pane closes it.
  const onCloseKeyDown = (event: KeyboardEvent<HTMLButtonElement>) => {
    if (event.key !== 'Escape') return;
    event.preventDefault();
    crew.closePane();
  };
  const onFocus = () => {
    focusInside.current = true;
  };
  const onBlur = (event: FocusEvent<HTMLElement>) => {
    const next = event.relatedTarget as Node | null;
    if (next && !aside.current?.contains(next)) focusInside.current = false;
  };

  const animateMode = useRef<string | null>(null);
  const modeChanged = Boolean(
    intent && animateMode.current !== null && animateMode.current !== intent.mode
  );
  useEffect(() => {
    animateMode.current = intent?.mode ?? null;
  }, [intent?.mode]);

  let content: ReactNode = null;
  if (shown) {
    const body =
      shown.mode === 'details' ? (
        <Tabs
          value={tab}
          onValueChange={(value) => crew.openPane({ mode: 'details', tab: value as DetailsTab })}
          className="crew-pane-body min-h-0 pb-4 text-text-default"
        >
          <TabsList
            aria-label={paneCopy.tabsLabel}
            className="sticky top-0 z-10 bg-background-default"
          >
            {TABS.map((value) => (
              <TabsTrigger key={value} value={value}>
                {paneCopy.tabs[value]}
              </TabsTrigger>
            ))}
          </TabsList>
          {TABS.map((value) => (
            // A panel is a tab stop (Radix), so it shows the quiet inset edge every keyboard
            // region draws when it takes focus, rather than nothing (Q2-68).
            <TabsContent key={value} value={value} className="mt-3 biorouter-focus-region">
              {tabs[value] ??
                (value === 'about' ? <AboutTab /> : value === 'members' ? <MembersTab /> : null)}
            </TabsContent>
          ))}
        </Tabs>
      ) : shown.mode === 'agent' ? (
        (agent ?? <AgentTaskPane />)
      ) : (
        <div className="crew-pane-body py-3">{chatAccess ?? null}</div>
      );
    content = (
      <PaneMode key={shown.mode} animate={modeChanged}>
        <div className="crew-pane-header flex shrink-0 items-center gap-1 border-b border-border-subtle bg-sidebar">
          {title && (
            <Button
              type="button"
              variant="ghost"
              size="sm"
              className="crew-cover-only no-drag"
              onClick={() => crew.closePane()}
            >
              <ArrowLeft aria-hidden="true" />
              {paneCopy.back(title)}
            </Button>
          )}
          <h2
            ref={heading}
            tabIndex={-1}
            className="min-w-0 flex-1 truncate px-2 text-label text-text-default"
          >
            {shown.mode === 'details' && title ? (
              <>
                <span className="crew-push-only">{title}</span>
                <span className="crew-cover-only">{paneCopy.coverTitle}</span>
              </>
            ) : (
              titleOf(shown, title)
            )}
          </h2>
          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                type="button"
                variant="ghost"
                shape="round"
                aria-label={closeNameOf(shown)}
                className="no-drag"
                onClick={() => crew.closePane()}
                onKeyDown={onCloseKeyDown}
              >
                <X aria-hidden="true" />
              </Button>
            </TooltipTrigger>
            <TooltipContent>{closeNameOf(shown)}</TooltipContent>
          </Tooltip>
        </div>
        {body}
      </PaneMode>
    );
  }

  return (
    <aside
      ref={aside}
      className={cn('crew-pane', className)}
      data-state={open ? 'open' : 'closed'}
      data-mode={shown?.mode}
      aria-label={intent ? landmarkOf(intent, title) : undefined}
      aria-hidden={open ? undefined : true}
      inert={!open}
      onKeyDown={onKeyDown}
      onFocus={onFocus}
      onBlur={onBlur}
      onAnimationEnd={(event) => {
        if (event.target === event.currentTarget && !open) setLeaving(null);
      }}
    >
      <div className="crew-pane-content">{content}</div>
    </aside>
  );
}
