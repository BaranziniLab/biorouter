/**
 * GENERATED — do not edit. Run `npm run themes` after changing a theme
 * definition in themes/*.theme.mjs.
 *
 * Carries the theme values that cannot live in CSS: xterm paints to a canvas
 * and react-syntax-highlighter takes a JS object, so neither can read a
 * custom property. Everything here is derived from the same definition that
 * produced the CSS token blocks, which is what keeps the two in step — these
 * values were previously hand-copied and drifted silently.
 */

export type SyntaxPalette = {
  plain: string;
  comment: string;
  keyword: string;
  string: string;
  number: string;
  func: string;
  type: string;
  operator: string;
  deleted: string;
  inserted: string;
};

export type TerminalPalette = {
  background: string;
  foreground: string;
  cursor: string;
  cursorAccent: string;
  selectionBackground: string;
  black: string;
  red: string;
  green: string;
  yellow: string;
  blue: string;
  magenta: string;
  cyan: string;
  white: string;
  brightBlack: string;
  brightRed: string;
  brightGreen: string;
  brightYellow: string;
  brightBlue: string;
  brightMagenta: string;
  brightCyan: string;
  brightWhite: string;
};

export type ThemeModeData = {
  /**
   * The brand mark's two inks for this family and mode. The boot splash paints
   * these before React exists, and BioRouterMark paints them after — reading
   * both from here is what stops the two disagreeing. Roche Limit used to flash
   * an orange mark that React then re-rendered in Parchment's coral.
   */
  mark: { navy: string; coral: string };
  syntax: SyntaxPalette;
  /** The surface the syntax palette is measured against (--background-code). */
  codeGround: string;
  /**
   * The paper well a fenced block sits in (--background-well). The palette is
   * held to AA here too, and on --background-default, by the generator.
   */
  wellGround: string;
  /**
   * Resolved literal values for the five semantic tokens a CSP-sandboxed
   * surface has to inline.
   *
   * A `srcdoc` iframe under `default-src 'none'` cannot reach the app
   * stylesheet, so `var(--text-default)` is empty inside it. The notebook and
   * spreadsheet previews build their own document and must write real hexes —
   * these, so the inlined values track the family instead of being frozen to
   * Parchment. Do not add hand-picked colours next to a preview; add the token
   * to SURFACE_TOKENS in scripts/generate-themes.mjs instead.
   */
  surface: {
    /** --background-default — the document ground. */
    background: string;
    /** --text-default — body ink, held to 4.5:1 against `background`. */
    foreground: string;
    /** --text-muted — secondary ink. */
    muted: string;
    /** --background-card — a table/cell ground that sits on `background`. */
    card: string;
    /** --border-subtle — hairlines and table cell rules. */
    border: string;
  };
  /** Which token the terminal dock paints. Families genuinely differ. */
  terminalGround: string;
  terminal: TerminalPalette;
};

export type GeneratedTheme = {
  id: string;
  label: string;
  swatch: string;
  light: ThemeModeData;
  dark: ThemeModeData;
};

export const GENERATED_THEMES = {
  parchment: {
    id: 'parchment',
    label: 'Parchment',
    swatch: '#cf6d47',
    light: {
      mark: { navy: '#052049', coral: '#b85a32' },
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
      codeGround: '#f5f5f3',
      wellGround: '#f5f5f3',
      surface: {
        background: '#ffffff',
        foreground: '#2a2520',
        muted: '#635c54',
        card: '#ffffff',
        border: '#e4e4e0',
      },
      terminalGround: '--background-muted',
      terminal: {
        background: '#f4f4f2',
        cursorAccent: '#f4f4f2',
        foreground: '#2d2a26',
        cursor: '#b85a32',
        selectionBackground: '#e4d9c3',
        black: '#2d2a26',
        red: '#b63f3f',
        green: '#22784f',
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
    },
    dark: {
      mark: { navy: '#18a3ac', coral: '#b85a32' },
      syntax: {
        plain: '#e8e1d2',
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
      codeGround: '#1b1b19',
      wellGround: '#232320',
      surface: {
        background: '#1b1b19',
        foreground: '#f4f0e6',
        muted: '#b0a892',
        card: '#1b1b19',
        border: '#302f2c',
      },
      terminalGround: '--background-muted',
      terminal: {
        background: '#232320',
        cursorAccent: '#232320',
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
    },
  },
  'alma-mater': {
    id: 'alma-mater',
    label: 'Alma Mater',
    swatch: '#14828c',
    light: {
      mark: { navy: '#052049', coral: '#16a0ac' },
      syntax: {
        plain: '#052049',
        comment: '#586780',
        keyword: '#0f388a',
        string: '#007242',
        number: '#8a5a00',
        func: '#6c247c',
        type: '#0e5258',
        operator: '#506380',
        deleted: '#c40d3e',
        inserted: '#007242',
      },
      codeGround: '#f5f5f3',
      wellGround: '#f5f5f3',
      surface: {
        background: '#ffffff',
        foreground: '#052049',
        muted: '#506380',
        card: '#ffffff',
        border: '#e4e4e0',
      },
      terminalGround: '--background-muted',
      terminal: {
        background: '#f4f4f2',
        cursorAccent: '#f4f4f2',
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
    },
    dark: {
      mark: { navy: '#18a3ac', coral: '#16a0ac' },
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
      codeGround: '#1b1b19',
      wellGround: '#232320',
      surface: {
        background: '#1b1b19',
        foreground: '#f2f3f4',
        muted: '#b4b9bf',
        card: '#1b1b19',
        border: '#302f2c',
      },
      terminalGround: '--background-muted',
      terminal: {
        background: '#232320',
        cursorAccent: '#232320',
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
    },
  },
  'roche-limit': {
    id: 'roche-limit',
    label: 'Roche Limit',
    swatch: '#ee6c1a',
    light: {
      mark: { navy: '#1f1e1c', coral: '#ee6c1a' },
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
      codeGround: '#f5f5f3',
      wellGround: '#f5f5f3',
      surface: {
        background: '#ffffff',
        foreground: '#1f1e1c',
        muted: '#5c5a55',
        card: '#ffffff',
        border: '#e4e4e0',
      },
      terminalGround: '--background-muted',
      terminal: {
        background: '#f4f4f2',
        cursorAccent: '#f4f4f2',
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
    },
    dark: {
      mark: { navy: '#ededea', coral: '#ee6c1a' },
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
      codeGround: '#1b1b19',
      wellGround: '#232320',
      surface: {
        background: '#1b1b19',
        foreground: '#ededea',
        muted: '#a5a39d',
        card: '#1b1b19',
        border: '#302f2c',
      },
      terminalGround: '--background-muted',
      terminal: {
        background: '#232320',
        cursorAccent: '#232320',
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
    },
  },
} as const satisfies Record<string, GeneratedTheme>;

export type ThemeFamilyId = keyof typeof GENERATED_THEMES;

/** Every family id, in definition order. The one list. */
export const THEME_FAMILY_IDS = Object.keys(GENERATED_THEMES) as ThemeFamilyId[];

/* ── the knowledge-graph palette ── */

/**
 * The credibility ring's keys: the six `CredibilityTier` values plus
 * `retracted`, which is a FLAG rather than a tier — a retracted source takes
 * the retracted colour and the continuous ring whatever its tier says, because
 * retraction is the more important fact.
 *
 * Declared here rather than imported from `api/types.gen` so this generated
 * module stays free of the API client's generation order;
 * `graphPalette.test.ts` asserts at COMPILE TIME that the two agree.
 */
export type GraphCredibilityKey =
  | 'peer_reviewed'
  | 'book'
  | 'preprint'
  | 'gray_lit'
  | 'web'
  | 'personal'
  | 'retracted';

export type GraphPalette = {
  /** The 28 curated fills, keyed by OKF type name. */
  types: Record<string, string>;
  /** Family name -> its members, in ladder order. */
  families: Record<string, { members: string[] }>;
  /** The seven ring hues. */
  credibility: Record<GraphCredibilityKey, string>;
  /**
   * The ring TREATMENT, which is the actual encoding: an arc count, a dashed
   * ring, or a continuous one. Hue rides along as the fast channel for
   * trichromats and carries nothing alone — a 1.6px stroke subtends ~2-3
   * arcmin, well inside the regime where the visual system reads luminance
   * only. `web` and `personal` share the dashed treatment because on the
   * canvas they ARE one category: not academic.
   */
  ringArcs: Record<GraphCredibilityKey, number | 'dashed' | 'solid'>;
  /** DR-11: the fixed chroma every hashed fallback colour takes. */
  fallbackChroma: number;
  /** DR-11: the four rungs a hashed fallback selects between. */
  /** R-05: the fallback's four OKLab lightnesses, not contrast rungs. */
  fallbackLightness: [number, number, number, number];
  /**
   * The surface every ratio above was solved against — the resolved
   * `--background-muted`, which is what the graph pane paints.
   *
   * RESOLVED, never authored, and per-mode. Consumers that need a canvas
   * fallback take it from here rather than restating a hex: a single light
   * value for a dual-mode quantity is how the boot mark once came to measure
   * 1.02:1 on every dark splash.
   */
  ground: string;
};

/**
 * The knowledge-graph node palette — 28 curated type fills, 7 credibility ring
 * hues, the shape map, and the DR-11 fallback parameters.
 *
 * ONE PAIR SERVES ALL THREE FAMILIES. That is not a simplification: the
 * generator resolves `--background-muted` in all six (family × mode) scopes
 * and refuses to emit unless the three light values are identical and the three
 * dark values are identical. If a family ever diverges, this constant moves
 * per-family — the generator will say so rather than letting the palette go
 * quietly wrong.
 *
 * These are NOT theme tokens and no CSS consumes them. A 2D canvas is the same
 * category of consumer as xterm and react-syntax-highlighter: it cannot read a
 * custom property, so the value has to arrive as a string. Adding them to
 * SEMANTIC_TOKENS would force every family to author 28 values it does not
 * need, and this design deliberately adds ZERO theme tokens.
 *
 * Every hex is a solved contrast ratio against `ground`, so the ladder
 * INVERTS between modes by construction: in light a higher rung is a darker
 * colour, in dark a lighter one. Same rung index, same relative position within
 * the family, opposite direction — which is what keeps a family readable in
 * both modes without a second authored table.
 */
export const GRAPH_PALETTE: { light: GraphPalette; dark: GraphPalette } = {
  light: {
    types: {
      Gene: '#a0b2ff',
      Variant: '#9190ed',
      SequenceFeature: '#8670cb',
      Structure: '#7952a8',
      Molecule: '#65cdb3',
      MolecularClass: '#3ab1a6',
      BiologicalPathway: '#009598',
      BiologicalFunction: '#007785',
      Anatomy: '#97c87c',
      CellType: '#67b073',
      Organism: '#32976b',
      Disease: '#ff8fb2',
      Phenotype: '#e77284',
      BiomedicalMeasure: '#ca5957',
      MethodOrProcedure: '#ac4228',
      Exposure: '#eba75f',
      SocialFactor: '#c5923a',
      Food: '#9e7e10',
      Device: '#7ec0ea',
      MaterialSample: '#7f9cd5',
      Publication: '#a9bdaf',
      Study: '#93ada8',
      Dataset: '#819ba0',
      Agent: '#758895',
      Population: '#6c7688',
      GeographicLocation: '#656376',
      Concept: '#5e5162',
      Other: '#54414b',
    },
    families: {
      Genomic: { members: ['Gene', 'Variant', 'SequenceFeature', 'Structure'] },
      'Molecular & process': {
        members: ['Molecule', 'MolecularClass', 'BiologicalPathway', 'BiologicalFunction'],
      },
      'Anatomy & organism': { members: ['Anatomy', 'CellType', 'Organism'] },
      Clinical: { members: ['Disease', 'Phenotype', 'BiomedicalMeasure', 'MethodOrProcedure'] },
      Exposome: { members: ['Exposure', 'SocialFactor', 'Food'] },
      Physical: { members: ['Device', 'MaterialSample'] },
      'Provenance & context': {
        members: [
          'Publication',
          'Study',
          'Dataset',
          'Agent',
          'Population',
          'GeographicLocation',
          'Concept',
          'Other',
        ],
      },
    },
    credibility: {
      peer_reviewed: '#1c619f',
      book: '#406e9d',
      preprint: '#5d7b9a',
      gray_lit: '#768290',
      web: '#a26227',
      personal: '#9f5c83',
      retracted: '#c04441',
    },
    ringArcs: {
      peer_reviewed: 4,
      book: 3,
      preprint: 2,
      gray_lit: 1,
      web: 'dashed',
      personal: 'dashed',
      retracted: 'solid',
    },
    fallbackChroma: 0.055,
    fallbackLightness: [0.7625, 0.7075, 0.6525, 0.5975],
    ground: '#f4f4f2',
  },
  dark: {
    types: {
      Gene: '#5060b6',
      Variant: '#7875d0',
      SequenceFeature: '#a08be8',
      Structure: '#c9a1fe',
      Molecule: '#007b66',
      MolecularClass: '#08968c',
      BiologicalPathway: '#34b0b3',
      BiologicalFunction: '#58cadb',
      Anatomy: '#4a772e',
      CellType: '#4d955a',
      Organism: '#51b285',
      Disease: '#a83d64',
      Phenotype: '#c9586a',
      BiomedicalMeasure: '#e87470',
      MethodOrProcedure: '#ff977d',
      Exposure: '#955900',
      SocialFactor: '#a97816',
      Food: '#b99837',
      Device: '#2d7096',
      MaterialSample: '#6582b8',
      Publication: '#3b4d41',
      Study: '#445c58',
      Dataset: '#526b6f',
      Agent: '#657885',
      Population: '#7c8698',
      GeographicLocation: '#9593a7',
      Concept: '#aea1b3',
      Other: '#c6b0bc',
    },
    families: {
      Genomic: { members: ['Gene', 'Variant', 'SequenceFeature', 'Structure'] },
      'Molecular & process': {
        members: ['Molecule', 'MolecularClass', 'BiologicalPathway', 'BiologicalFunction'],
      },
      'Anatomy & organism': { members: ['Anatomy', 'CellType', 'Organism'] },
      Clinical: { members: ['Disease', 'Phenotype', 'BiomedicalMeasure', 'MethodOrProcedure'] },
      Exposome: { members: ['Exposure', 'SocialFactor', 'Food'] },
      Physical: { members: ['Device', 'MaterialSample'] },
      'Provenance & context': {
        members: [
          'Publication',
          'Study',
          'Dataset',
          'Agent',
          'Population',
          'GeographicLocation',
          'Concept',
          'Other',
        ],
      },
    },
    credibility: {
      peer_reviewed: '#5fa1e4',
      book: '#6291c2',
      preprint: '#6583a3',
      gray_lit: '#6d7986',
      web: '#ba783f',
      personal: '#b67299',
      retracted: '#e1625d',
    },
    ringArcs: {
      peer_reviewed: 4,
      book: 3,
      preprint: 2,
      gray_lit: 1,
      web: 'dashed',
      personal: 'dashed',
      retracted: 'solid',
    },
    fallbackChroma: 0.055,
    fallbackLightness: [0.6175, 0.6725, 0.7275, 0.7825],
    ground: '#232320',
  },
};
