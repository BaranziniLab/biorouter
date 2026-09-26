/**
 * Roche Limit — theme definition.
 *
 * THIS FILE IS THE SOURCE OF TRUTH for the Roche Limit theme. Everything else
 * (the CSS token blocks, the syntax palette, the terminal palette, the picker
 * entry, the boot splash) is generated from it by scripts/generate-themes.mjs.
 * Do not edit the generated regions — edit here and re-run `npm run themes`.
 */

/** @type {import('../scripts/lib/theme-contract.mjs').ThemeDefinition} */
export default {
  id: 'roche-limit',
  label: 'Roche Limit',
  swatch: '#ee6c1a',

  // Which token the terminal dock paints. SHARED across families — all three
  // point at `--background-muted`, in both modes.
  terminalGround: {
    light: '--background-muted',
    dark: '--background-muted',
  },

  light: {
    tokens: {
      'background-accent': '#ee6c1a',
      'background-accent-hover': '#de6110',
      'border-accent': '#ee6c1a',
      'text-accent': '#ae4700',
      'text-on-accent': '#1f1e1c',
      'accent-bar': '#d95b08',
      // ── SHARED NEUTRALS — THE REFERENCE SET ──────────────────────────────
      // Every surface, grey and border from here down is the one neutral set
      // all three families wear, and Roche Limit is where it came from: its
      // values were adopted wholesale by Parchment and Alma Mater rather than
      // averaged, so this block is unchanged by the unification. Editing a
      // neutral here now moves all three families — mirror it into
      // parchment.theme.mjs, alma-mater.theme.mjs and main.css's :root/.dark.
      'background-app': '#ffffff',
      // "White as the main colour" (roche-limit-theme.md §1): the page is pure
      // white and `background-muted` #f4f4f2 is the intermediate panel. The gap
      // between the two is guarded — see check-contrast.mjs, "a step off the
      // canvas"; the other two families used to collapse it to zero.
      'background-canvas': '#ffffff',
      'background-default': '#ffffff',
      'background-card': '#ffffff',
      'background-muted': '#f4f4f2',
      'background-code': '#f5f5f3',
      'background-well': '#f5f5f3',
      'background-medium': '#ecece9',
      'background-strong': '#dcdcd8',
      'background-inverse': '#1f1e1c',
      'background-danger': '#c4232b',
      'background-success': '#0f7150',
      'background-info': '#0a69bc',
      'background-warning': '#6e6300',
      'text-on-status': '#ffffff',
      'border-subtle': '#e4e4e0',
      'border-strong': '#d2d2cd',
      'border-input': '#c9c9c3',
      'border-default': '#e4e4e0',
      'border-danger': '#c4232b',
      'border-success': '#0f7150',
      'border-warning': '#6e6300',
      'border-info': '#0a69bc',
      'text-default': '#1f1e1c',
      'text-muted': '#5c5a55',
      'text-subtle': '#69675f',
      'text-inverse': '#ffffff',
      // Status inks are held to 4.5:1 on their OWN WASH (`--wash-X`, the hue at
      // 22% over the ground), not just on white: a Note paints the pair, and the
      // old stops measured 3.5–4.5:1 there (live QA T-18). Darkened in OKLCH
      // lightness at a fixed hue; `check-contrast.mjs` asserts every pair.
      'text-danger': '#a8061c',
      'text-success': '#026546',
      'text-warning': '#625800',
      'text-info': '#0258a2',
      ring: '#5c5a55',
      'background-focus': '#e0e0dc',
      'border-focus': '#6b6963',
      'heat-0': '#eeeeea',
      'heat-1': '#fadcc0',
      'heat-2': '#f6bc8c',
      'heat-3': '#ee8b45',
      'heat-4': '#c4560e',
      sidebar: '#f7f7f5',
      'sidebar-foreground': '#1f1e1c',
      'sidebar-icon': '#d95b08',
      'sidebar-hover': '#efefec',
      'sidebar-active': '#eaeae6',
      'sidebar-accent': '#efefec',
      'sidebar-accent-foreground': '#1f1e1c',
      'sidebar-border': '#e7e7e3',
      'sidebar-ring': '#5c5a55',
      // ── PERSON AVATAR HUES (D-AVATAR) ─────────────────────────────────────
      // Eight fill + initials-ink pairs, chosen by the person's canonical
      // @username so a display name cannot borrow a colour. SHARED across the
      // three families, like the neutrals: one person keeps one colour on every
      // device whatever family it runs, and `themeNeutrals.test.ts` holds the
      // three copies together. OKLCH hues 25°–330°, about 45° apart, at one lightness
      // (fills L 0.885, inks L 0.39) so no hue reads heavier than another; the ink
      // measures 6.7–7.0:1 on its fill (`check-contrast.mjs` asserts 4.5).
      'avatar-hue-1-bg': '#ffcbc5', // red
      'avatar-hue-1-fg': '#752725',
      'avatar-hue-2-bg': '#fccfab', // orange
      'avatar-hue-2-fg': '#673702',
      'avatar-hue-3-bg': '#e7d9a5', // amber
      'avatar-hue-3-fg': '#534400',
      'avatar-hue-4-bg': '#bde6bd', // green
      'avatar-hue-4-fg': '#11531a',
      'avatar-hue-5-bg': '#a3e9e3', // teal
      'avatar-hue-5-fg': '#04504d',
      'avatar-hue-6-bg': '#b5dfff', // blue
      'avatar-hue-6-fg': '#034a6f',
      'avatar-hue-7-bg': '#d4d5ff', // violet
      'avatar-hue-7-fg': '#41397d',
      'avatar-hue-8-bg': '#f4c9ef', // pink
      'avatar-hue-8-fg': '#642b61',
      'shadow-default':
        '0px 1px 3px 0px rgba(31, 30, 28, 0.07), 0px 0px 1px 0px rgba(31, 30, 28, 0.13)',
      'shadow-composer':
        '0px 2px 6px -1px rgba(31, 30, 28, 0.09), 0px 1px 2px 0px rgba(31, 30, 28, 0.05)',
      'shadow-popover':
        '0px 8px 24px 0px rgba(31, 30, 28, 0.11), 0px 0px 1px 0px rgba(31, 30, 28, 0.16)',
      'shadow-modal':
        '0px 22px 60px -18px rgba(31, 30, 28, 0.22), 0px 8px 24px -18px rgba(31, 30, 28, 0.16), 0px 0px 0px 1px rgba(31, 30, 28, 0.05)',
      scrim: 'rgba(31, 30, 28, 0.18)',
    },
    raw: {
      'color-coral-400': '#f2955a',
      'color-coral-500': '#ee6c1a',
      'color-coral-600': '#ae4700',
      'color-coral-700': '#8f3a00',
      'color-neutral-50': '#fafaf9',
      'color-neutral-100': '#f4f4f2',
      'color-neutral-200': '#e4e4e0',
      'color-neutral-300': '#d2d2cd',
      'color-neutral-400': '#a9a7a1',
      'color-neutral-500': '#84827c',
      'color-neutral-600': '#5c5a55',
      'color-neutral-700': '#3e3d39',
      'color-neutral-800': '#2c2c29',
      'color-neutral-900': '#1b1b19',
      'color-neutral-950': '#131312',
      'color-red-100': '#ff9592',
      'color-red-200': '#e85e62',
      'color-red-600': '#c4232b',
      'color-blue-100': '#70b8ff',
      'color-blue-200': '#4a97ee',
      'color-blue-700': '#0a69bc',
      'color-green-100': '#3dd68c',
      'color-green-200': '#22b071',
      'color-green-600': '#0f7150',
      'color-yellow-100': '#f2e06b',
      'color-yellow-200': '#d8c23a',
      'color-yellow-700': '#6e6300',
    },
    syntax: {
      plain: '#1f1e1c',
      comment: '#3f6e6e',
      keyword: '#0a7a32',
      string: '#b02121',
      number: '#0f6e38',
      func: '#1849b8',
      type: '#0f6e38',
      operator: '#7024b0',
      deleted: '#c4232b',
      inserted: '#12805c',
    },
    terminal: {
      foreground: '#1f1e1c',
      cursor: '#d95b08',
      selectionBackground: '#fadbbb',
      black: '#1f1e1c',
      red: '#c4232b',
      green: '#0f7150',
      yellow: '#6e6300',
      blue: '#0a69bc',
      magenta: '#7024b0',
      cyan: '#3f6e6e',
      white: '#69675f',
      brightBlack: '#5c5a55',
      brightRed: '#a8161e',
      brightGreen: '#0a5e42',
      brightYellow: '#5a5100',
      brightBlue: '#08579c',
      brightMagenta: '#5c1d91',
      brightCyan: '#2f5a5a',
      brightWhite: '#1f1e1c',
    },
    mark: {
      navy: '#1f1e1c',
      coral: '#ee6c1a',
      track: '#e4e4e0',
    },
  },
  dark: {
    tokens: {
      'background-accent': '#ee6c1a',
      'background-accent-hover': '#f27f30',
      'border-accent': '#ee6c1a',
      'text-accent': '#f2955a',
      'text-on-accent': '#131312',
      'accent-bar': '#ee6c1a',
      // ── SHARED NEUTRALS — THE REFERENCE SET ──────────────────────────────
      'background-app': '#131312',
      // Dark flips polarity: canvas #131312, card #1B1B19 (roche-limit-theme.md
      // §5.x). Warm-neutral, not pure black, so bright orange does not halate.
      // This ladder — canvas darkest, cards a step up — is now the one ladder.
      // Parchment dark and Alma Mater dark both used to invert it (canvas
      // LIGHTER than cards); a shared set cannot carry two contradictory orders.
      'background-canvas': '#131312',
      'background-default': '#1b1b19',
      'background-card': '#1b1b19',
      'background-muted': '#232320',
      'background-code': '#1b1b19',
      'background-well': '#232320',
      'background-medium': '#2c2c29',
      'background-strong': '#3a3a36',
      'background-inverse': '#ededea',
      'background-danger': '#ff9592',
      'background-success': '#3dd68c',
      'background-info': '#70b8ff',
      'background-warning': '#f2e06b',
      'text-on-status': '#131312',
      'border-subtle': '#302f2c',
      'border-strong': '#3e3d39',
      'border-input': '#4a4945',
      'border-default': '#302f2c',
      'border-danger': '#ff9592',
      'border-success': '#3dd68c',
      'border-warning': '#f2e06b',
      'border-info': '#70b8ff',
      'text-default': '#ededea',
      'text-muted': '#a5a39d',
      'text-subtle': '#9c9a93',
      'text-inverse': '#131312',
      'text-danger': '#ff9592',
      'text-success': '#3dd68c',
      'text-warning': '#f2e06b',
      'text-info': '#70b8ff',
      ring: '#a5a39d',
      'background-focus': '#35342f',
      'border-focus': '#9c9a93',
      'heat-0': '#1e1d1b',
      'heat-1': '#4a2a0e',
      'heat-2': '#7a4413',
      'heat-3': '#b45f18',
      'heat-4': '#ee6c1a',
      sidebar: '#171716',
      'sidebar-foreground': '#ededea',
      'sidebar-icon': '#ee6c1a',
      'sidebar-hover': '#232320',
      'sidebar-active': '#2e2e2a',
      'sidebar-accent': '#232320',
      'sidebar-accent-foreground': '#ededea',
      'sidebar-border': '#2a2a27',
      'sidebar-ring': '#a5a39d',
      // Person avatar hues (D-AVATAR): the light set's hues at fill L 0.39 and
      // ink L 0.925, a step off every dark ground; ink 7.4–7.9:1 on its fill.
      // Orange and amber are calmer than the rest (QA Q3-61): at full chroma
      // they read as brown and olive beside indigo, teal and plum. They sit at
      // chroma 0.073 / 0.058 (OKLCH 58° / 85°), which is Carol's measured pair
      // with orange nudged 5° toward red, because her pair measured ΔE00 7.4
      // apart and `check-contrast.mjs` holds every two fills 8 apart; this pair
      // is 10.0. Disc on the dark canvas 1.89 and 2.01; ink 7.88 and 7.45:1.
      'avatar-hue-1-bg': '#6b302d', // red
      'avatar-hue-1-fg': '#feddda',
      'avatar-hue-2-bg': '#623a19', // orange
      'avatar-hue-2-fg': '#fbe2cc',
      'avatar-hue-3-bg': '#564520', // amber
      'avatar-hue-3-fg': '#f1e6c9',
      'avatar-hue-4-bg': '#245127', // green
      'avatar-hue-4-fg': '#d4efd4',
      'avatar-hue-5-bg': '#01504d', // teal
      'avatar-hue-5-fg': '#c5f0ed',
      'avatar-hue-6-bg': '#084a6e', // blue
      'avatar-hue-6-fg': '#d0eafe',
      'avatar-hue-7-bg': '#413d70', // violet
      'avatar-hue-7-fg': '#e3e3ff',
      'avatar-hue-8-bg': '#5e335a', // pink
      'avatar-hue-8-fg': '#f8dcf5',
      'shadow-default': '0px 1px 3px 0px rgba(0, 0, 0, 0.25), 0px 0px 1px 0px rgba(0, 0, 0, 0.35)',
      'shadow-composer':
        '0px 2px 10px -1px rgba(0, 0, 0, 0.45), 0px 1px 3px 0px rgba(0, 0, 0, 0.3)',
      'shadow-popover': '0px 8px 24px 0px rgba(0, 0, 0, 0.4), 0px 0px 1px 0px rgba(0, 0, 0, 0.5)',
      'shadow-modal':
        '0px 22px 64px -16px rgba(0, 0, 0, 0.62), 0px 8px 26px -18px rgba(0, 0, 0, 0.5), 0px 0px 0px 1px rgba(255, 255, 255, 0.055)',
      scrim: 'rgba(0, 0, 0, 0.48)',
    },
    raw: {
      'color-coral-400': '#f2955a',
      'color-coral-500': '#ee6c1a',
      'color-coral-600': '#ae4700',
      'color-coral-700': '#8f3a00',
      'color-neutral-50': '#fafaf9',
      'color-neutral-100': '#f4f4f2',
      'color-neutral-200': '#e4e4e0',
      'color-neutral-300': '#d2d2cd',
      'color-neutral-400': '#a9a7a1',
      'color-neutral-500': '#84827c',
      'color-neutral-600': '#5c5a55',
      'color-neutral-700': '#3e3d39',
      'color-neutral-800': '#2c2c29',
      'color-neutral-900': '#1b1b19',
      'color-neutral-950': '#131312',
      'color-red-100': '#ff9592',
      'color-red-200': '#e85e62',
      'color-red-600': '#c4232b',
      'color-blue-100': '#70b8ff',
      'color-blue-200': '#4a97ee',
      'color-blue-700': '#0a69bc',
      'color-green-100': '#3dd68c',
      'color-green-200': '#22b071',
      'color-green-600': '#0f7150',
      'color-yellow-100': '#f2e06b',
      'color-yellow-200': '#d8c23a',
      'color-yellow-700': '#6e6300',
    },
    syntax: {
      plain: '#ededea',
      comment: '#7fa3a3',
      keyword: '#6fcb78',
      string: '#ff8f8f',
      number: '#84d089',
      func: '#7fbef7',
      type: '#84d089',
      operator: '#d9a0ff',
      deleted: '#ff9592',
      inserted: '#3dd68c',
    },
    terminal: {
      foreground: '#ededea',
      cursor: '#ee6c1a',
      selectionBackground: '#452201',
      black: '#3a3a36',
      red: '#ff9592',
      green: '#3dd68c',
      yellow: '#f2e06b',
      blue: '#70b8ff',
      magenta: '#d9a0ff',
      cyan: '#7fa3a3',
      white: '#a5a39d',
      brightBlack: '#9c9a93',
      brightRed: '#ffb3b0',
      brightGreen: '#6fe5a6',
      brightYellow: '#f7ec9a',
      brightBlue: '#9ccdff',
      brightMagenta: '#e6bcff',
      brightCyan: '#a0c0c0',
      brightWhite: '#ffffff',
    },
    mark: {
      navy: '#ededea',
      coral: '#ee6c1a',
      track: '#302f2c',
    },
  },
};
