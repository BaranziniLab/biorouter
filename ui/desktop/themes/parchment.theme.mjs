/**
 * Parchment — theme definition.
 *
 * THIS FILE IS THE SOURCE OF TRUTH for the Parchment theme. Everything else
 * (the CSS token blocks, the syntax palette, the terminal palette, the picker
 * entry, the boot splash) is generated from it by scripts/generate-themes.mjs.
 * Do not edit the generated regions — edit here and re-run `npm run themes`.
 */

/** @type {import('../scripts/lib/theme-contract.mjs').ThemeDefinition} */
export default {
  id: 'parchment',
  label: 'Parchment',
  swatch: '#cf6d47',
  isBase: true,

  // Which token the terminal dock paints. SHARED across families: all three
  // point at `--background-muted`, in both modes. Parchment used to paint
  // `--background-code` in dark, which was a real difference while the two
  // tokens held different per-family values; now that the neutral set is one
  // set, a dock that painted a different token would be the ONLY surface still
  // varying by family. Parchment's dark ANSI palette is re-verified against the
  // muted ground by the generator, which is what makes this a measurement
  // rather than the assumption the contract file warns about.
  terminalGround: {
    light: '--background-muted',
    dark: '--background-muted',
  },

  light: {
    tokens: {
      'background-accent': '#b85a32',
      'background-accent-hover': '#a94f2a',
      'border-accent': '#b85a32',
      'text-accent': '#b85a32',
      'text-on-accent': '#ffffff',
      'accent-bar': '#cf6d47',
      // ── SHARED NEUTRALS ──────────────────────────────────────────────────
      // Every surface, grey and border below is the one set all three families
      // wear. Parchment's identity is its warm INK and its dark-orange ACCENT.
      // Do not reintroduce a warm ground here; retune a text token instead.
      // The base family's CSS is hand-authored in main.css — these values
      // mirror it, and the two must be changed together.
      'background-app': '#ffffff',
      'background-canvas': '#ffffff',
      'background-default': '#ffffff',
      'background-card': '#ffffff',
      'background-muted': '#f4f4f2',
      'background-code': '#f5f5f3',
      'background-well': '#f5f5f3',
      'background-medium': '#ecece9',
      'background-strong': '#dcdcd8',
      // The tooltip fill. Warm near-black, not `#000`. This was per family —
      // each set it to its own `text-default` — but a tooltip is a SURFACE, so
      // all three now paint it and place their own `text-inverse` on top.
      'background-inverse': '#1f1e1c',
      'background-danger': '#b3261e',
      'background-success': '#1f7a3d',
      'background-info': '#1e5fbf',
      'background-warning': '#8a5a00',
      'text-on-status': '#ffffff',
      'border-subtle': '#e4e4e0',
      'border-strong': '#d2d2cd',
      'border-input': '#c9c9c3',
      'border-default': '#e4e4e0',
      'border-danger': '#b3261e',
      'border-success': '#1f7a3d',
      'border-warning': '#8a5a00',
      'border-info': '#1e5fbf',
      'text-default': '#2a2520',
      'text-muted': '#635c54',
      'text-subtle': '#6e6760',
      'text-inverse': '#ffffff',
      // Status inks are held to 4.5:1 on their OWN WASH (`--wash-X`, the hue at
      // 22% over the ground), not just on white: a Note paints the pair, and the
      // old stops measured 3.5–4.5:1 there (live QA T-18). Darkened in OKLCH
      // lightness at a fixed hue; `check-contrast.mjs` asserts every pair.
      'text-danger': '#a51a14',
      'text-success': '#05662d',
      'text-warning': '#7a5000',
      'text-info': '#1053b0',
      ring: '#5c5a55',
      'background-focus': '#e0e0dc',
      'border-focus': '#6b6963',
      // heat-0 is the empty-day fill — a neutral, shared. 1–4 are the family's
      // own accent ramp and stay warm orange.
      'heat-0': '#eeeeea',
      'heat-1': '#e9c9ab',
      'heat-2': '#dda27a',
      'heat-3': '#c6774c',
      'heat-4': '#a04a27',
      sidebar: '#f7f7f5',
      'sidebar-foreground': '#2a2520',
      'sidebar-icon': '#2a2520',
      'sidebar-hover': '#efefec',
      'sidebar-active': '#eaeae6',
      'sidebar-accent': '#efefec',
      'sidebar-accent-foreground': '#2a2520',
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
    syntax: {
      plain: '#2a2520',
      comment: '#6f6659',
      keyword: '#a94f2a',
      string: '#22784f',
      number: '#8a5a00',
      func: '#255fb5',
      type: '#7847b8',
      operator: '#6e6760',
      deleted: '#b3261e',
      inserted: '#1f7a3d',
    },
    terminal: {
      foreground: '#2d2a26',
      cursor: '#b85a32',
      selectionBackground: '#e4d9c3',
      black: '#2d2a26',
      red: '#b63f3f',
      green: '#22784f',
      // `yellow` and `cyan` were darkened by ~2.5% (from #9b6818 and #107e89)
      // when the neutral set was shared: the light dock ground moved from
      // Parchment's cream #faf8f3 to the shared #f4f4f2, and both stops landed
      // at 4.35/4.37:1 — just under AA. They now clear 4.55/4.60. This is the
      // sanctioned repair for a shared-ground regression: retune the FAMILY'S
      // OWN INK, never reintroduce a family-specific background.
      yellow: '#976517',
      blue: '#255fb5',
      magenta: '#7847b8',
      cyan: '#107a85',
      white: '#574f46',
      brightBlack: '#6f6659',
      brightRed: '#d45252',
      brightGreen: '#1f7a3d',
      brightYellow: '#8a5a00',
      brightBlue: '#2f75d6',
      brightMagenta: '#9462d6',
      brightCyan: '#1f9aa6',
      brightWhite: '#2d2a26',
    },
    mark: {
      navy: '#052049',
      coral: '#b85a32',
      // The splash progress track is a hairline, so it takes the shared
      // `border-subtle` rather than a family grey.
      track: '#e4e4e0',
    },
  },
  dark: {
    tokens: {
      'background-accent': '#e8895f',
      'background-accent-hover': '#f0a07c',
      'border-accent': '#e8895f',
      'text-accent': '#e8895f',
      'text-on-accent': '#16120c',
      'accent-bar': '#e8895f',
      // ── SHARED NEUTRALS ──────────────────────────────────────────────────
      // Parchment dark used to invert the surface ladder — canvas #282217
      // LIGHTER than its #16120c cards — and Alma Mater did the same in navy.
      // Both are gone: one shared neutral set cannot carry two contradictory
      // orders, and the shared one is Roche Limit's (canvas darkest, cards a
      // step up). This is the single largest visible change of the unification.
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
      'background-danger': '#f07575',
      'background-success': '#7ac87c',
      'background-info': '#7aabf5',
      'background-warning': '#f0c84a',
      'text-on-status': '#16120c',
      'border-subtle': '#302f2c',
      'border-strong': '#3e3d39',
      'border-input': '#4a4945',
      'border-default': '#302f2c',
      'border-danger': '#f07575',
      'border-success': '#7ac87c',
      'border-warning': '#f0c84a',
      'border-info': '#7aabf5',
      'text-default': '#f4f0e6',
      'text-muted': '#b0a892',
      'text-subtle': '#9c937b',
      'text-inverse': '#16120c',
      // Status inks are held to 4.5:1 on their OWN WASH; see the light block.
      'text-danger': '#ff8f8c',
      'text-success': '#7ac87c',
      'text-warning': '#f0c84a',
      'text-info': '#80b0fa',
      ring: '#a5a39d',
      'background-focus': '#35342f',
      'border-focus': '#9c9a93',
      'heat-0': '#1e1d1b',
      'heat-1': '#4a3524',
      'heat-2': '#7a4d2e',
      'heat-3': '#b0653a',
      'heat-4': '#e8895f',
      sidebar: '#171716',
      'sidebar-foreground': '#f4f0e6',
      'sidebar-icon': '#f4f0e6',
      'sidebar-hover': '#232320',
      'sidebar-active': '#2e2e2a',
      'sidebar-accent': '#232320',
      'sidebar-accent-foreground': '#f4f0e6',
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
    syntax: {
      plain: '#e8e1d2',
      // #8d8266 until the paper well: 4.53:1 on the code ground but 4.14:1 on
      // the #232320 well a fenced block sits in. Lifted along its own hue
      // (the sanctioned repair — retune the family's ink, never its ground).
      comment: '#958a6c',
      keyword: '#e8895f',
      string: '#7fbf6a',
      number: '#d9a441',
      func: '#8fb8e8',
      type: '#b98ad6',
      operator: '#b0a892',
      deleted: '#f07575',
      inserted: '#7ac87c',
    },
    terminal: {
      foreground: '#e8e1d2',
      cursor: '#e8895f',
      selectionBackground: '#403928',
      black: '#3a3324',
      red: '#e2665c',
      green: '#7fbf6a',
      yellow: '#d9a441',
      blue: '#6f9fd8',
      magenta: '#b98ad6',
      cyan: '#5fb8b8',
      white: '#d4cab6',
      brightBlack: '#8d8266',
      brightRed: '#f0857b',
      brightGreen: '#9ad686',
      brightYellow: '#ecc063',
      brightBlue: '#8fb8e8',
      brightMagenta: '#d0a6e8',
      brightCyan: '#7fd0d0',
      brightWhite: '#e8e1d2',
    },
    mark: {
      navy: '#18a3ac',
      coral: '#b85a32',
      // Shared `border-subtle`, like the light track.
      track: '#302f2c',
    },
  },
};
