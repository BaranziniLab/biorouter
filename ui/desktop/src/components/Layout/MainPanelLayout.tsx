import React from 'react';

// The default ground is `bg-background-canvas` — NOT `bg-background-muted`.
// `--background-canvas` is the main-panel/page token: this panel IS the page,
// and the sidebar is the one surface that deliberately steps away from it.
// Painting the panel `muted` made the whole canvas read grey and collapsed the
// sidebar/canvas distinction the designs are built on. Each family sets
// `background-canvas` per mode, so one class stays correct across
// Parchment / Alma Mater / Roche Limit without per-view logic.
export const MainPanelLayout: React.FC<{
  children: React.ReactNode;
  removeTopPadding?: boolean;
  backgroundColor?: string;
}> = ({ children, removeTopPadding = false, backgroundColor = 'bg-background-canvas' }) => {
  // We deliberately use `h-full` here, not `h-dvh`. `h-dvh` (dynamic viewport
  // height) forces the layout to viewport size regardless of the parent — which
  // breaks small chat surfaces: a 420px-tall chat pane would render its panel
  // at ~1050px and push the ChatInput off-screen. With `h-full`, the panel
  // fills its container, whether that's the viewport (standalone /pair, etc.)
  // or a chat pane's rect.
  //
  // `removeTopPadding` is what every band route passes (spec 3.10): the 44px
  // band starts at y=0 and replaces the 32px drag strip there, so a reserve
  // above it would be dead space. A route that keeps the strip keeps the
  // reserve, `--titlebar-drag-height` rather than a 32px literal. The reserve
  // is authored CSS (`.biorouter-main-panel[data-top-reserve]` in `main.css`)
  // rather than a newly written arbitrary utility (the class-scanning trap,
  // CLAUDE.md "Desktop shell geometry"), and that rule also drops it on its
  // own when the panel holds a `PageHeader` band, so a view that has moved
  // onto the band sits at y=0 even before it passes the prop.
  return (
    <div
      className={`biorouter-main-panel flex flex-col ${backgroundColor} h-full min-w-0 min-h-0`}
      data-top-reserve={removeTopPadding ? undefined : ''}
    >
      {children}
    </div>
  );
};
