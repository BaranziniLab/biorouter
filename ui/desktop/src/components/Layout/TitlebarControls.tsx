import type { CSSProperties, PointerEvent } from 'react';
import { NewWindow } from '../icons/app-icons';
import { Button } from '../ui/button';
import { SidebarTrigger } from '../ui/sidebar';

const MACOS_TRAFFIC_LIGHT_RESERVE = 100;
const NON_MACOS_TITLEBAR_INSET = 16;
/**
 * Width of the floating control strip: TWO 32px controls (sidebar toggle +
 * new window). Keep this in lockstep with the buttons rendered below — the
 * reserve derived from it is what stops the session title/tabs from sliding
 * under the strip (and, on macOS, under the traffic lights) when the sidebar
 * is collapsed. TitlebarControls.test.tsx asserts the resulting numbers.
 */
const TITLEBAR_CONTROL_STRIP_WIDTH = 64;
const TITLEBAR_CONTROL_GAP = 8;

/**
 * Below this width the sidebar overlays rather than pushes, so the session
 * title / tab strip must reserve room for the floating control strip. Lives
 * here with the rest of the titlebar geometry because BaseChat and the chat tab
 * strip both derive their left padding from it and must not drift apart.
 */
export { SIDEBAR_COMPACT_WIDTH as SIDEBAR_COMPACT_TITLE_WIDTH } from './yieldLadder';

export const TITLEBAR_CONTROL_RESERVE_PROPERTY = '--biorouter-titlebar-control-reserve';

export function getTitlebarControlInset(isMacOS: boolean): number {
  return isMacOS ? MACOS_TRAFFIC_LIGHT_RESERVE : NON_MACOS_TITLEBAR_INSET;
}

export function getTitlebarControlReserve(isMacOS: boolean): number {
  return getTitlebarControlInset(isMacOS) + TITLEBAR_CONTROL_STRIP_WIDTH + TITLEBAR_CONTROL_GAP;
}

export function getSessionTitlePadding(
  isCompactSidebarOverlayOpen: boolean,
  reserveTitlebarControls: boolean
): string {
  if (isCompactSidebarOverlayOpen) return 'calc(var(--sidebar-width) + 8px)';
  if (reserveTitlebarControls) {
    // Derived, never hardcoded: AppLayout sets the custom property from the
    // same function, so the fallback can never drift away from the real strip
    // width the way a literal did. macOS is the fallback case because that is
    // the only platform where the reserve is load-bearing (traffic lights).
    return `var(${TITLEBAR_CONTROL_RESERVE_PROPERTY}, ${getTitlebarControlReserve(true)}px)`;
  }
  return '16px';
}

interface TitlebarControlsProps {
  hidden: boolean;
  isMacOS: boolean;
  onNewWindow: () => void;
}

export function TitlebarControls({ hidden, isMacOS, onNewWindow }: TitlebarControlsProps) {
  if (hidden) return null;

  const stopTitlebarDrag = (event: PointerEvent<HTMLDivElement>) => event.stopPropagation();

  return (
    <div
      data-testid="titlebar-controls"
      className="no-drag pointer-events-auto absolute top-2 z-[190] isolate flex items-center"
      style={
        {
          left: getTitlebarControlInset(isMacOS),
          WebkitAppRegion: 'no-drag',
        } as CSSProperties
      }
      onPointerDown={stopTitlebarDrag}
    >
      {/* Both controls hover on the strong fill, the step every fill takes on
          the band's recessed ground (R-27; the summary and terminal toggles
          press on the same token). Both name themselves in the app tooltip
          (`title`, upgraded by AppTooltipLayer), so the pair reads alike. */}
      <SidebarTrigger
        data-testid="titlebar-sidebar-toggle"
        size="sm"
        shape="round"
        title="Toggle sidebar"
        className="no-drag text-text-muted hover:!bg-background-fill-strong hover:text-text-default"
      />
      <Button
        data-testid="titlebar-new-window"
        onClick={onNewWindow}
        className="no-drag text-text-muted hover:!bg-background-fill-strong hover:text-text-default"
        variant="ghost"
        size="sm"
        shape="round"
        title="Start a new chat in a new window"
      >
        <NewWindow className="h-4 w-4" />
      </Button>
    </div>
  );
}
