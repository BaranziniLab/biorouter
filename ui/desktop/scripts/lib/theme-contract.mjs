/**
 * The theme token contract — the single written-down answer to "what does a
 * theme have to define?".
 *
 * Before this existed the answer was folklore: you copied the previous family's
 * blocks and hoped. Every estimate of the vocabulary size was wrong (the design
 * discussion assumed ~45; it is 58 semantic + 27 raw + 17 structural), which is
 * how two separate refactor proposals mispriced themselves threefold.
 *
 * Shrinking this list is a real saving: it is what a NEW family has to author.
 * The Astryx sweep removed four consumerless entries — `sidebar-primary`,
 * `sidebar-primary-foreground` and the two `shadow-modal-chrome-*` no-ops —
 * which is eight fewer hand-picked values per family across the two modes.
 *
 * Anything listed REQUIRED here must be present in every theme definition or
 * the generator refuses to emit. Anything DERIVED is computed from other tokens
 * and must NOT be written by hand — those are precisely the values that used to
 * be duplicated across main.css, codeTheme.ts, InAppTerminalDock.tsx and
 * index.html, and that silently drifted.
 */

/**
 * How many person-avatar hues there are (D-AVATAR, Crew live QA round 2, carol
 * F4). A people-heavy surface — a channel's member stack, a timeline where six
 * people post — read as a row of identical grey tiles, so "who is this" had to
 * be answered by reading every name. Each hue is a PAIR, a tile fill and the
 * initials ink on it, and `check-contrast.mjs` holds the ink to 4.5:1 on its
 * own fill in every family and mode (initials are 11–13px, so small text).
 *
 * An avatar picks its hue from the person's canonical `@username`, never from
 * the display name, so a self-chosen name cannot borrow somebody else's colour.
 * The hue is a recognition aid, not an identity proof: the `@username` stays on
 * screen. The component sets `data-hue="1"…"8"` on `.biorouter-avatar`, and the
 * rules in `main.css` paint the pair.
 */
export const AVATAR_HUE_COUNT = 8;

/** `avatar-hue-1-bg`, `avatar-hue-1-fg`, … `avatar-hue-8-fg`, in that order. */
export const AVATAR_HUE_TOKENS = Array.from({ length: AVATAR_HUE_COUNT }, (_, i) => [
  `avatar-hue-${i + 1}-bg`,
  `avatar-hue-${i + 1}-fg`,
]).flat();

/** Semantic colour tokens. Every family declares all of these, in both modes. */
export const SEMANTIC_TOKENS = [
  // accent
  'background-accent',
  'background-accent-hover',
  'border-accent',
  'text-accent',
  'text-on-accent',
  'accent-bar',
  // surfaces
  //
  // `background-app` is the WINDOW ground (what `body` paints, behind
  // everything). `background-canvas` is the MAIN PANEL ground — the page the
  // conversation, the hub and every top-level view are drawn on, and the
  // surface the sidebar deliberately steps away from. They are separate
  // because the families disagree about whether the page is pure white:
  // Parchment's canvas carries the warm tint while its window ground is white,
  // and Parchment dark inverts the ladder entirely (canvas lighter than cards).
  // Collapsing the two would force one family's ladder onto the others.
  'background-app',
  'background-canvas',
  'background-default',
  'background-card',
  'background-muted',
  'background-code',
  // The faint well set INTO a document surface — a fenced block or inline code
  // in the artifact panel's paper, a notebook cell's source. It is not
  // `background-code`: that one is a panel a hair off the page in light and the
  // card ground itself in dark, and a well that equals the page it sits on is
  // no well. Shared, and every syntax palette is measured on it.
  'background-well',
  'background-medium',
  'background-strong',
  'background-inverse',
  // status fills + their ink
  'background-danger',
  'background-success',
  'background-info',
  'background-warning',
  'text-on-status',
  // borders
  'border-subtle',
  'border-strong',
  'border-input',
  'border-default',
  'border-danger',
  'border-success',
  'border-warning',
  'border-info',
  // text
  'text-default',
  'text-muted',
  'text-subtle',
  'text-inverse',
  'text-danger',
  'text-success',
  'text-warning',
  'text-info',
  // focus
  'ring',
  'background-focus',
  'border-focus',
  // heatmap
  'heat-0',
  'heat-1',
  'heat-2',
  'heat-3',
  'heat-4',
  // sidebar
  'sidebar',
  'sidebar-foreground',
  'sidebar-icon',
  'sidebar-hover',
  'sidebar-active',
  'sidebar-accent',
  'sidebar-accent-foreground',
  'sidebar-border',
  'sidebar-ring',
  // person avatars — eight fill + ink pairs; see AVATAR_HUE_COUNT above
  ...AVATAR_HUE_TOKENS,
  // scrim — see note below
  'scrim',
  // shadows (compound strings, not colours)
  'shadow-default',
  'shadow-composer',
  'shadow-popover',
  'shadow-modal',
];

/**
 * `--scrim` is new. The modal and diagnostics overlays were hardcoded
 * `rgba(32,25,15,0.18)` — Parchment's warm brown — outside the token layer
 * entirely, with a single hand-written override retinting Alma Mater light.
 * Roche Limit, added later, therefore wore Parchment's brown scrim. Promoting
 * it to a token is what stops the next family repeating that.
 */
export const SHADOW_TOKENS = new Set([
  'shadow-default',
  'shadow-composer',
  'shadow-popover',
  'shadow-modal',
]);

/** Tokens whose value is a colour and can therefore be contrast-checked. */
export const COLOUR_TOKENS = SEMANTIC_TOKENS.filter((t) => !SHADOW_TOKENS.has(t) && t !== 'scrim');

/**
 * Raw palette remaps. Mode-independent by design — they recolour the stray
 * direct `neutral-*` / `coral-*` utility usages so nothing is left with the
 * previous family's bias. The semantic layer above is authoritative; these only
 * catch components that reach past it.
 */
export const RAW_TOKENS = [
  'color-coral-400',
  'color-coral-500',
  'color-coral-600',
  'color-coral-700',
  'color-neutral-50',
  'color-neutral-100',
  'color-neutral-200',
  'color-neutral-300',
  'color-neutral-400',
  'color-neutral-500',
  'color-neutral-600',
  'color-neutral-700',
  'color-neutral-800',
  'color-neutral-900',
  'color-neutral-950',
  'color-red-100',
  'color-red-200',
  'color-red-600',
  'color-blue-100',
  'color-blue-200',
  'color-blue-700',
  'color-green-100',
  'color-green-200',
  'color-green-600',
  'color-yellow-100',
  'color-yellow-200',
  'color-yellow-700',
];

/** Prism syntax stops. Measured against the family's own code ground. */
export const SYNTAX_STOPS = [
  'plain',
  'comment',
  'keyword',
  'string',
  'number',
  'func',
  'type',
  'operator',
  'deleted',
  'inserted',
];

/**
 * xterm ANSI-16 plus the four chrome slots. `background` is NOT here: it is
 * DERIVED from the family's own `terminalGround` token rather than hardcoded,
 * because hardcoding one answer would silently re-ground a terminal under a
 * palette tuned for a different surface.
 *
 * The three families currently AGREE — all declare `--background-muted` for
 * both modes — and that is a measurement, not a rule. It was not always so:
 * Parchment's dark dock painted `--background-code` until the families were
 * moved onto one shared set of neutrals, and its dark ANSI palette had to be
 * re-measured against the ground it moved to. So the field stays per-family and
 * this stays derived: read what a family declares, never assume, in either
 * direction. Agreement today is not a licence to collapse the field tomorrow.
 *
 * `cursorAccent` is likewise derived — it is always the ground.
 */
export const TERMINAL_STOPS = [
  'foreground',
  'cursor',
  'selectionBackground',
  'black',
  'red',
  'green',
  'yellow',
  'blue',
  'magenta',
  'cyan',
  'white',
  'brightBlack',
  'brightRed',
  'brightGreen',
  'brightYellow',
  'brightBlue',
  'brightMagenta',
  'brightCyan',
  'brightWhite',
];

/**
 * Per-slot contrast floors for the ANSI palette, with the reason recorded for
 * every relaxation. Anything not listed owes the full 4.5:1 on its own ground.
 *
 * WHY THE BRIGHT SLOTS ARE 3:1. On a LIGHT terminal ground "bright" and "AA"
 * are in direct conflict: the convention is that a bright variant is *lighter*
 * than its base, and lighter on a light ground means less contrast. Parchment's
 * bright stops sat at 3.18–4.27:1 — every one of them below the 4.5 its own
 * doc comment claimed, undetected because the terminal palettes had no test at
 * all. Forcing 4.5 would either invert the convention (bright darker than base)
 * or collapse the pairs together: brightCyan would have to darken to #007e8a,
 * a hair from cyan's #107e89, making the two indistinguishable. So the bright
 * slots hold the 3:1 graphical/large-text floor, which they all clear, and the
 * BASE slots — which carry the bulk of terminal output — hold the full 4.5.
 * This is a deliberate, documented relaxation, not an oversight.
 */
export const TERMINAL_FLOORS = {
  black: { floor: 1.0, why: 'ANSI dim slot — a lifted ground by convention, not text' },
  brightBlack: { floor: 3.0, why: 'ANSI dim slot — used for dimmed/comment output' },
  brightRed: { floor: 3.0, why: 'bright variant — see TERMINAL_FLOORS note' },
  brightGreen: { floor: 3.0, why: 'bright variant — see TERMINAL_FLOORS note' },
  brightYellow: { floor: 3.0, why: 'bright variant — see TERMINAL_FLOORS note' },
  brightBlue: { floor: 3.0, why: 'bright variant — see TERMINAL_FLOORS note' },
  brightMagenta: { floor: 3.0, why: 'bright variant — see TERMINAL_FLOORS note' },
  brightCyan: { floor: 3.0, why: 'bright variant — see TERMINAL_FLOORS note' },
  brightWhite: { floor: 3.0, why: 'bright variant — see TERMINAL_FLOORS note' },
  cursor: { floor: 3.0, why: 'non-text UI element (SC 1.4.11)' },
  selectionBackground: { floor: 1.0, why: 'a surface, not a foreground' },
};

/** Kept for callers that only need the never-checked slots. */
export const TERMINAL_DIM_SLOTS = {
  black: TERMINAL_FLOORS.black.why,
};

/**
 * The brand mark's inks, plus the splash progress track. Named `mark`, not
 * `splash`: these drive BOTH the pre-React boot splash AND the in-app
 * <BioRouterMark>, and calling them "splash" is what let the two drift apart in
 * the first place. (The <BioRouterWordmark> deliberately does NOT read these —
 * see the note in that component.) `bg` is derived from --background-muted.
 */
export const MARK_STOPS = ['navy', 'coral', 'track'];

/** @deprecated alias kept so an out-of-tree import does not break silently. */
export const SPLASH_STOPS = MARK_STOPS;

/**
 * Structural tokens: identical across every family, declared once in `:root`.
 * A theme definition must NOT restate them.
 */
export const STRUCTURAL_TOKENS = [
  'radius-full',
  'motion-fast',
  'motion-base',
  'motion-slow',
  'ease-out',
  'ease-in',
  'row-height',
  'z-base',
  'z-sticky',
  'z-dropdown',
  'z-overlay',
  'z-modal',
  'z-modal-dropdown',
  'z-toast',
  'heat-cell',
  'heat-gap',
  'heat-labels',
  // The knowledge workspace's geometry (ui-spec §6.2). Structural for the same
  // reason `--row-height` is: it is the same measurement under every family, and
  // nothing else stops a fourth family declaring a rail width of its own.
  'measure-graph',
  'knowledge-rail-sources',
  'knowledge-rail-detail',
  'knowledge-subject-height',
  'knowledge-filter-height',
  'knowledge-pane-two-col',
  'knowledge-pane-legend-card',
  'knowledge-pane-full-filters',
  'knowledge-pane-three-col',
  'knowledge-pane-short',
];

/** Validate one mode of a theme definition. Returns an array of problems. */
export function validateMode(id, mode, def) {
  const problems = [];
  const has = (obj, key) => obj && Object.prototype.hasOwnProperty.call(obj, key);

  for (const t of SEMANTIC_TOKENS) {
    if (!has(def.tokens, t)) problems.push(`${id}.${mode}: missing token --${t}`);
  }
  for (const s of SYNTAX_STOPS) {
    if (!has(def.syntax, s)) problems.push(`${id}.${mode}: missing syntax stop "${s}"`);
  }
  for (const s of TERMINAL_STOPS) {
    if (!has(def.terminal, s)) problems.push(`${id}.${mode}: missing terminal stop "${s}"`);
  }
  for (const s of MARK_STOPS) {
    if (!has(def.mark, s)) problems.push(`${id}.${mode}: missing mark value "${s}"`);
  }
  if (has(def.terminal, 'background')) {
    problems.push(`${id}.${mode}: terminal.background is DERIVED from terminalGround — remove it`);
  }
  for (const t of STRUCTURAL_TOKENS) {
    if (has(def.tokens, t)) {
      problems.push(`${id}.${mode}: --${t} is structural and must not be themed`);
    }
  }
  return problems;
}

/** Validate a whole theme definition. */
export function validateTheme(theme) {
  const problems = [];
  for (const field of ['id', 'label', 'swatch', 'terminalGround']) {
    if (!theme[field]) problems.push(`${theme.id ?? '?'}: missing "${field}"`);
  }
  for (const mode of ['light', 'dark']) {
    if (!theme[mode]) {
      problems.push(`${theme.id}: missing "${mode}" mode`);
      continue;
    }
    problems.push(...validateMode(theme.id, mode, theme[mode]));
    const ground = theme.terminalGround?.[mode];
    if (ground && !SEMANTIC_TOKENS.includes(ground.replace(/^--/, ''))) {
      problems.push(`${theme.id}.${mode}: terminalGround "${ground}" is not a known token`);
    }
  }
  // Parchment is the base family: it supplies the bare :root/.dark defaults and
  // therefore does not carry raw-palette remaps.
  if (!theme.isBase) {
    for (const t of RAW_TOKENS) {
      if (!Object.prototype.hasOwnProperty.call(theme.light.raw ?? {}, t)) {
        problems.push(`${theme.id}.light: missing raw palette --${t}`);
      }
    }
  }
  return problems;
}
