/**
 * Alma Mater — theme definition.
 *
 * THIS FILE IS THE SOURCE OF TRUTH for the Alma Mater theme. Everything else
 * (the CSS token blocks, the syntax palette, the terminal palette, the picker
 * entry, the boot splash) is generated from it by scripts/generate-themes.mjs.
 * Do not edit the generated regions — edit here and re-run `npm run themes`.
 */

/** @type {import('../scripts/lib/theme-contract.mjs').ThemeDefinition} */
export default {
  id: 'alma-mater',
  label: 'Alma Mater',
  swatch: '#14828c',

  // Which token the terminal dock paints. SHARED across families — all three
  // point at `--background-muted`, in both modes.
  terminalGround: {
    light: '--background-muted',
    dark: '--background-muted',
  },

  light: {
    tokens: {
      'background-accent': '#14828c',
      'background-accent-hover': '#0e5258',
      'border-accent': '#14828c',
      'text-accent': '#14828c',
      'text-on-accent': '#ffffff',
      'accent-bar': '#16a0ac',
      // ── SHARED NEUTRALS ──────────────────────────────────────────────────
      // Alma Mater used to run a COOL blue-grey scaffolding (`muted` #f2f3f4,
      // `medium` #e1e3e5, `strong` #d1d3d3, `border-subtle` #e1e3e5) to sit
      // with its navy ink. That is gone: neutrals are one shared set, and the
      // family is now UCSF navy ink + UCSF teal accent on the same ground as
      // its siblings. White page, grey sidebar — the two-tone canvas of the
      // design mockups (tokens doc §5a) — survives, because the shared set
      // keeps a white canvas and a lifted `sidebar`.
      'background-app': '#ffffff',
      'background-canvas': '#ffffff',
      'background-default': '#ffffff',
      'background-card': '#ffffff',
      'background-muted': '#f4f4f2',
      'background-code': '#f5f5f3',
      'background-well': '#f5f5f3',
      'background-medium': '#ecece9',
      'background-strong': '#dcdcd8',
      // A tooltip is a surface, so it is shared rather than set to this
      // family's own `text-default` navy.
      'background-inverse': '#1f1e1c',
      'background-danger': '#e61048',
      'background-success': '#007242',
      'background-info': '#0f388a',
      'background-warning': '#8a5a00',
      'text-on-status': '#ffffff',
      'border-subtle': '#e4e4e0',
      'border-strong': '#d2d2cd',
      'border-input': '#c9c9c3',
      'border-default': '#e4e4e0',
      'border-danger': '#c40d3e',
      'border-success': '#007242',
      'border-warning': '#8a5a00',
      'border-info': '#0f388a',
      'text-default': '#052049',
      'text-muted': '#506380',
      'text-subtle': '#586780',
      'text-inverse': '#ffffff',
      // Status inks are held to 4.5:1 on their OWN WASH (`--wash-X`, the hue at
      // 22% over the ground), not just on white: a Note paints the pair, and the
      // old stops measured 3.5–4.5:1 there (live QA T-18). Darkened in OKLCH
      // lightness at a fixed hue; `check-contrast.mjs` asserts every pair.
      'text-danger': '#a8032f',
      'text-success': '#02653a',
      'text-warning': '#7a5000',
      'text-info': '#0f388a',
      ring: '#5c5a55',
      'background-focus': '#e0e0dc',
      'border-focus': '#6b6963',
      // heat-0 is the empty-day fill — a neutral, shared. 1–4 are this
      // family's own teal ramp.
      'heat-0': '#eeeeea',
      'heat-1': '#b4e2e8',
      'heat-2': '#60d0da',
      'heat-3': '#16a0ac',
      'heat-4': '#0e5258',
      sidebar: '#f7f7f5',
      'sidebar-foreground': '#052049',
      'sidebar-icon': '#14828c',
      'sidebar-hover': '#efefec',
      'sidebar-active': '#eaeae6',
      'sidebar-accent': '#efefec',
      'sidebar-accent-foreground': '#052049',
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
      // Elevation and the scrim are neutral scaffolding and therefore shared.
      // Alma Mater used to cast every alpha in UCSF navy (rgba(5, 32, 73, …));
      // a full-screen navy scrim is a background, and backgrounds no longer
      // vary by family.
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
      'color-coral-400': '#60d0da',
      'color-coral-500': '#16a0ac',
      'color-coral-600': '#14828c',
      'color-coral-700': '#0e5258',
      // The neutral ramp is SHARED — identical to the base `@theme` block and
      // to Roche Limit's. It is restated here only because the generator
      // requires every non-base family to declare the full raw palette; there
      // is nothing family-specific left in these eleven values.
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
      'color-red-100': '#f5768a',
      'color-red-200': '#e85268',
      'color-red-600': '#c40d3e',
      'color-blue-100': '#7fb3e6',
      'color-blue-200': '#4f8fd6',
      'color-blue-700': '#0f388a',
      'color-green-100': '#5fbf74',
      'color-green-200': '#38a552',
      'color-green-600': '#007242',
      'color-yellow-100': '#feb80a',
      'color-yellow-200': '#e0a400',
      'color-yellow-700': '#8a5a00',
    },
    syntax: {
      plain: '#052049',
      comment: '#586780',
      // #0f388a until the preview paper: navy on the #052049 navy ink, so a
      // keyword differed from an identifier by weight alone. #0b67a8 separates
      // by hue — 5.97:1 on #ffffff, 5.47:1 on the #f5f5f3 well.
      keyword: '#0b67a8',
      string: '#007242',
      number: '#8a5a00',
      func: '#6c247c',
      type: '#0e5258',
      operator: '#506380',
      deleted: '#c40d3e',
      inserted: '#007242',
    },
    terminal: {
      foreground: '#052049',
      cursor: '#0e5258',
      selectionBackground: '#d8d9da',
      black: '#052049',
      red: '#c40d3e',
      green: '#007242',
      yellow: '#8a5a00',
      blue: '#0f388a',
      magenta: '#6c247c',
      cyan: '#0e5258',
      white: '#506380',
      brightBlack: '#586780',
      brightRed: '#d0143f',
      brightGreen: '#1f7a3d',
      brightYellow: '#8a5a00',
      brightBlue: '#255fb5',
      brightMagenta: '#8a1fa0',
      brightCyan: '#106a72',
      brightWhite: '#052049',
    },
    mark: {
      navy: '#052049',
      coral: '#16a0ac',
      // Shared `border-subtle` — the splash progress track is a hairline.
      track: '#e4e4e0',
    },
  },
  dark: {
    tokens: {
      'background-accent': '#60d0da',
      'background-accent-hover': '#8ae0e8',
      'border-accent': '#60d0da',
      'text-accent': '#60d0da',
      'text-on-accent': '#052049',
      'accent-bar': '#60d0da',
      // ── SHARED NEUTRALS ──────────────────────────────────────────────────
      // Alma Mater dark used to be navy all the way down, and ran a LIGHTER
      // canvas (#0d2a50) than its cards (#08213f) — a pin that was explicitly
      // "the shipped, user-preferred appearance". Both are gone: the neutral
      // set is one set, and it runs canvas-darkest / cards-a-step-up. The
      // family keeps its navy where navy belongs — the INK — and its teal
      // accent. This is the single largest visible change of the unification.
      'background-app': '#131312',
      'background-canvas': '#131312',
      'background-default': '#1b1b19',
      'background-card': '#1b1b19',
      'background-muted': '#232320',
      'background-code': '#1b1b19',
      'background-well': '#232320',
      'background-medium': '#2c2c29',
      'background-strong': '#3a3a36',
      'background-inverse': '#ededea',
      'background-danger': '#f5768a',
      'background-success': '#5fbf74',
      'background-info': '#7fb3e6',
      'background-warning': '#feb80a',
      'text-on-status': '#052049',
      'border-subtle': '#302f2c',
      'border-strong': '#3e3d39',
      'border-input': '#4a4945',
      'border-default': '#302f2c',
      'border-danger': '#f5768a',
      'border-success': '#5fbf74',
      'border-warning': '#feb80a',
      'border-info': '#7fb3e6',
      'text-default': '#f2f3f4',
      'text-muted': '#b4b9bf',
      'text-subtle': '#909aa6',
      'text-inverse': '#052049',
      // Status inks are held to 4.5:1 on their OWN WASH; see the light block.
      'text-danger': '#fd90a0',
      'text-success': '#63c277',
      'text-warning': '#feb80a',
      'text-info': '#7fb3e6',
      ring: '#a5a39d',
      'background-focus': '#35342f',
      'border-focus': '#9c9a93',
      'heat-0': '#1e1d1b',
      'heat-1': '#0e5258',
      'heat-2': '#14828c',
      'heat-3': '#16a0ac',
      'heat-4': '#60d0da',
      sidebar: '#171716',
      'sidebar-foreground': '#f2f3f4',
      'sidebar-icon': '#60d0da',
      'sidebar-hover': '#232320',
      'sidebar-active': '#2e2e2a',
      'sidebar-accent': '#232320',
      'sidebar-accent-foreground': '#f2f3f4',
      'sidebar-border': '#2a2a27',
      'sidebar-ring': '#a5a39d',
      // Person avatar hues (D-AVATAR): the light set's hues at fill L 0.39 and
      // ink L 0.925, a step off every dark ground; ink 7.5–7.9:1 on its fill.
      'avatar-hue-1-bg': '#6b302d', // red
      'avatar-hue-1-fg': '#feddda',
      'avatar-hue-2-bg': '#65380a', // orange
      'avatar-hue-2-fg': '#fee0c9',
      'avatar-hue-3-bg': '#524402', // amber
      'avatar-hue-3-fg': '#efe7c5',
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
      'color-coral-400': '#60d0da',
      'color-coral-500': '#16a0ac',
      'color-coral-600': '#14828c',
      'color-coral-700': '#0e5258',
      // Shared neutral ramp — see the note on the light block.
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
      'color-red-100': '#f5768a',
      'color-red-200': '#e85268',
      'color-red-600': '#c40d3e',
      'color-blue-100': '#7fb3e6',
      'color-blue-200': '#4f8fd6',
      'color-blue-700': '#0f388a',
      'color-green-100': '#5fbf74',
      'color-green-200': '#38a552',
      'color-green-600': '#007242',
      'color-yellow-100': '#feb80a',
      'color-yellow-200': '#e0a400',
      'color-yellow-700': '#8a5a00',
    },
    syntax: {
      plain: '#e1e3e5',
      comment: '#8a93a6',
      keyword: '#7fb3e6',
      string: '#6fc084',
      number: '#e0a44a',
      func: '#c58ad6',
      type: '#5cc6d0',
      operator: '#b4b9bf',
      deleted: '#f5768a',
      inserted: '#5fbf74',
    },
    terminal: {
      foreground: '#e1e3e5',
      cursor: '#8ae0e8',
      selectionBackground: '#163864',
      black: '#1e477f',
      red: '#f5768a',
      green: '#5fbf74',
      yellow: '#feb80a',
      blue: '#7fb3e6',
      magenta: '#c58ad6',
      cyan: '#5cc6d0',
      white: '#b4b9bf',
      brightBlack: '#909aa6',
      brightRed: '#ff8fa0',
      brightGreen: '#7fd08f',
      brightYellow: '#ffca4a',
      brightBlue: '#a3c9f0',
      brightMagenta: '#d7a5e8',
      brightCyan: '#7fd8e0',
      brightWhite: '#f2f3f4',
    },
    mark: {
      navy: '#18a3ac',
      coral: '#16a0ac',
      // Shared `border-subtle`, like the light track.
      track: '#302f2c',
    },
  },
};
