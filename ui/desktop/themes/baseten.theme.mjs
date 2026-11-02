/**
 * Baseten — theme definition.
 *
 * THIS FILE IS THE SOURCE OF TRUTH for the Baseten theme. Everything else
 * (the CSS token blocks, the syntax palette, the terminal palette, the picker
 * entry, the boot splash) is generated from it by scripts/generate-themes.mjs.
 * Do not edit the generated regions — edit here and re-run `npm run themes`.
 *
 * Baseten's brand: neutrals carry the layout, one green accent marks state.
 * Green #19e76e is a FILL colour, never small text, so it paints the primary
 * button (with black ink on it), switches, the heatmap and the dark-mode rail;
 * on light grounds the accent's text, edge and rail use the deeper greens
 * (#009735 / #007a2b) that hold contrast on white. Status success and info use
 * the brand's dark green and dark blue; danger and warning keep the shared
 * hues because the brand defines none. Syntax uses only the green, blue and
 * pink families. The brand's typeface, tracking and 2px corners are authored
 * in main.css under `:root[data-theme='baseten']`, not here — the contract
 * carries colour only.
 *
 * Neutrals (surfaces, greys, borders, avatars) are the shared set every family
 * wears, copied from roche-limit.theme.mjs and held equal by
 * themeNeutrals.test.ts; the brand's green-tinted greys are not used, so a
 * Baseten window sits on the same ground as the other three families.
 */

/** @type {import('../scripts/lib/theme-contract.mjs').ThemeDefinition} */
export default {
  id: 'baseten',
  label: 'Baseten',
  swatch: '#19e76e',

  // Which token the terminal dock paints. SHARED across families — every family
  // points at `--background-muted`, in both modes.
  terminalGround: {
    light: '--background-muted',
    dark: '--background-muted',
  },

  light: {
    tokens: {
      'background-accent': '#19e76e',
      'background-accent-hover': '#14d062',
      'border-accent': '#009735',
      'text-accent': '#007a2b',
      'text-on-accent': '#000000',
      'accent-bar': '#009735',
      // ── SHARED NEUTRALS ──────────────────────────────────────────────────
      // Every surface, grey and border from here down is the one neutral set
      // all families wear; roche-limit.theme.mjs is the reference. Change it
      // there and mirror it here, never here alone.
      'background-app': '#ffffff',
      // The brand's white canvas: the page is pure white and
      // `background-muted` #f4f4f2 is the intermediate panel.
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
      'background-success': '#007a2b',
      'background-info': '#0065ad',
      'background-warning': '#6e6300',
      'text-on-status': '#ffffff',
      'border-subtle': '#e4e4e0',
      'border-strong': '#d2d2cd',
      'border-input': '#c9c9c3',
      'border-default': '#e4e4e0',
      'border-danger': '#c4232b',
      'border-success': '#007a2b',
      'border-warning': '#6e6300',
      'border-info': '#0065ad',
      'text-default': '#1c1c1b',
      'text-muted': '#575d56',
      'text-subtle': '#666b65',
      'text-inverse': '#ffffff',
      // Status inks are held to 4.5:1 on their OWN WASH (`--wash-X`, the hue at
      // 22% over the ground), not just on white: a Note paints the pair, and the
      // old stops measured 3.5–4.5:1 there (live QA T-18). Darkened in OKLCH
      // lightness at a fixed hue; `check-contrast.mjs` asserts every pair.
      'text-danger': '#a8061c',
      'text-success': '#005c20',
      'text-warning': '#625800',
      'text-info': '#00589a',
      ring: '#5c5a55',
      'background-focus': '#e0e0dc',
      'border-focus': '#6b6963',
      'heat-0': '#eeeeea',
      'heat-1': '#a6efbb',
      'heat-2': '#19e76e',
      'heat-3': '#00b844',
      'heat-4': '#007a2b',
      sidebar: '#f7f7f5',
      'sidebar-foreground': '#1c1c1b',
      // Nav icons wear the row's resting ink, the muted label ink, as in
      // Parchment and Alma Mater: one ink with the labels and the chat glyphs
      // below (V1-R2-5). The accent is held for the active rail and the lock
      // badge (redesign principle 7).
      'sidebar-icon': '#575d56',
      'sidebar-hover': '#efefec',
      'sidebar-active': '#eaeae6',
      'sidebar-accent': '#efefec',
      'sidebar-accent-foreground': '#1c1c1b',
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
      'color-coral-400': '#5cf096',
      'color-coral-500': '#19e76e',
      'color-coral-600': '#009735',
      'color-coral-700': '#007a2b',
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
      plain: '#1c1c1b',
      comment: '#575d56',
      keyword: '#007a2b',
      string: '#ac0eb2',
      number: '#004a80',
      func: '#0065ad',
      type: '#005c20',
      operator: '#7d0a81',
      deleted: '#c4232b',
      inserted: '#007a2b',
    },
    terminal: {
      foreground: '#1c1c1b',
      cursor: '#007a2b',
      selectionBackground: '#d7face',
      black: '#1c1c1b',
      red: '#c4232b',
      green: '#007a2b',
      yellow: '#6e6300',
      blue: '#0065ad',
      magenta: '#ac0eb2',
      cyan: '#3f6e6e',
      white: '#666b65',
      brightBlack: '#575d56',
      brightRed: '#a8161e',
      brightGreen: '#005c20',
      brightYellow: '#5a5100',
      brightBlue: '#00589a',
      brightMagenta: '#8a0b8f',
      brightCyan: '#2f5a5a',
      brightWhite: '#1c1c1b',
    },
    mark: {
      navy: '#1c1c1b',
      coral: '#19e76e',
      track: '#e4e4e0',
    },
  },
  dark: {
    tokens: {
      'background-accent': '#19e76e',
      'background-accent-hover': '#4bf08e',
      'border-accent': '#19e76e',
      'text-accent': '#7df2ad',
      'text-on-accent': '#000000',
      'accent-bar': '#19e76e',
      // ── SHARED NEUTRALS — THE REFERENCE SET ──────────────────────────────
      'background-app': '#131312',
      // Dark flips polarity: canvas #131312, card #1B1B19 — the shared ladder,
      // canvas darkest and cards a step up.
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
      'background-success': '#5cf096',
      'background-info': '#6dbbff',
      'background-warning': '#f2e06b',
      'text-on-status': '#131312',
      'border-subtle': '#302f2c',
      'border-strong': '#3e3d39',
      'border-input': '#4a4945',
      'border-default': '#302f2c',
      'border-danger': '#ff9592',
      'border-success': '#5cf096',
      'border-warning': '#f2e06b',
      'border-info': '#6dbbff',
      'text-default': '#f3f6f2',
      'text-muted': '#b4bcb2',
      'text-subtle': '#a3aba1',
      'text-inverse': '#131312',
      'text-danger': '#ff9592',
      'text-success': '#5cf096',
      'text-warning': '#f2e06b',
      'text-info': '#6dbbff',
      ring: '#a5a39d',
      'background-focus': '#35342f',
      'border-focus': '#9c9a93',
      'heat-0': '#32312e', // one step up from #1e1d1b: zero days read on the dark card (redesign 2.3)
      'heat-1': '#1d5f33', // held 1.2:1 off heat-0 so a quiet day still reads
      'heat-2': '#0f9a45',
      'heat-3': '#13c45c',
      'heat-4': '#19e76e',
      sidebar: '#171716',
      'sidebar-foreground': '#f3f6f2',
      'sidebar-icon': '#b4bcb2', // the muted label ink, as in light
      'sidebar-hover': '#232320',
      'sidebar-active': '#2e2e2a',
      'sidebar-accent': '#232320',
      'sidebar-accent-foreground': '#f3f6f2',
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
      'color-coral-400': '#5cf096',
      'color-coral-500': '#19e76e',
      'color-coral-600': '#009735',
      'color-coral-700': '#007a2b',
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
      plain: '#f3f6f2',
      comment: '#a3aba1',
      keyword: '#5cf096',
      string: '#ffa9fd',
      number: '#b5dcff',
      func: '#6dbbff',
      type: '#a6f5c3',
      operator: '#ffd0fe',
      deleted: '#ff9592',
      inserted: '#5cf096',
    },
    terminal: {
      foreground: '#f3f6f2',
      cursor: '#19e76e',
      selectionBackground: '#0f3d1f',
      black: '#3a3a36',
      red: '#ff9592',
      green: '#5cf096',
      yellow: '#f2e06b',
      blue: '#6dbbff',
      magenta: '#ffa9fd',
      cyan: '#7fa3a3',
      white: '#b4bcb2',
      brightBlack: '#a3aba1',
      brightRed: '#ffb3b0',
      brightGreen: '#8ff5b6',
      brightYellow: '#f7ec9a',
      brightBlue: '#9ccdff',
      brightMagenta: '#ffc8fe',
      brightCyan: '#a0c0c0',
      brightWhite: '#ffffff',
    },
    mark: {
      navy: '#f3f6f2',
      coral: '#19e76e',
      track: '#302f2c',
    },
  },
};
