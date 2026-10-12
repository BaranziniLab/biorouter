/**
 * Shared theme-token parsing and cascade resolution.
 *
 * One implementation, used by every guard that needs to know what a token
 * actually resolves to in a given theme: check-contrast.mjs, the codegen's
 * regression check, and the duplication assertions. Previously each of these
 * reimplemented the parser and the WCAG maths, which is how a syntax palette
 * came to be verified against a surface the app never painted.
 *
 * The family list is DISCOVERED from the stylesheet, not hardcoded. Adding a
 * theme family therefore requires no edit here and no edit in the guards — the
 * new family is picked up and audited automatically.
 */

/**
 * The stylesheet with every comment blanked to spaces (newlines kept), so
 * offsets and line numbers still match the source.
 *
 * Comments are prose, and prose in this file names braces and declarations:
 * the `:root` block's note on `--measure-chat` cites
 * `components/sessions/{SessionListView,…}.tsx`, and the nested-block strip in
 * `parseDecls` read that `{…}` as a block and deleted everything from the top of
 * `:root` to it. Every token declared above that comment (`--border-emphasized`,
 * the motion ladder, the control geometry) was invisible to the guards, which
 * reported a token placed there as UNRESOLVED rather than measuring it. A
 * comment can also spell `--name: value;`, which would have been parsed as a
 * declaration. Blanking first closes both.
 */
export function blankComments(css) {
  return css.replace(/\/\*[\s\S]*?\*\//g, (comment) => comment.replace(/[^\n]/g, ' '));
}

/** Body of every block whose selector satisfies `test` (comments blanked). */
export function blocks(css, test) {
  const code = blankComments(css);
  const out = [];
  const re = /(^|\n)([^\n{}]+)\{/g;
  let m;
  while ((m = re.exec(code))) {
    const selector = m[2].trim();
    if (!test(selector)) continue;
    let depth = 1;
    let i = re.lastIndex;
    for (; i < code.length && depth > 0; i++) {
      if (code[i] === '{') depth++;
      else if (code[i] === '}') depth--;
    }
    out.push({ selector, body: code.slice(re.lastIndex, i - 1) });
  }
  return out;
}

export function parseDecls(body) {
  const o = {};
  // Strip nested blocks (e.g. @keyframes inside @theme) before reading decls.
  const flat = body.replace(/[^{}]*\{[^{}]*\}/g, '');
  for (const m of flat.matchAll(/(--[\w-]+)\s*:\s*([^;]+);/g)) o[m[1]] = m[2].trim();
  return o;
}

const merge = (...bs) => Object.assign({}, ...bs.map((b) => parseDecls(b.body)));

/**
 * Discover every theme family declared in the stylesheet.
 *
 * Matches `[data-theme='x']` only on the two token-block selectors, so the
 * per-family component overrides (e.g. the modal scrim rule) do not register
 * as families.
 */
export function discoverFamilies(css) {
  const found = new Set();
  for (const { selector } of blocks(css, () => true)) {
    const m = selector.match(/^(?::root|\.dark)\[data-theme='([^']+)'\]$/);
    if (m) found.add(m[1]);
  }
  return [...found].sort();
}

/**
 * Build the resolved scope for every family × mode in the stylesheet, plus the
 * bare Parchment default under the id `parchment`.
 *
 * The cascade is modelled the way the browser applies it:
 *   light  = :root                 < :root[data-theme=X]
 *   dark   = :root < .dark         < :root[data-theme=X] < .dark[data-theme=X]
 *
 * NOTE ON SOURCE ORDER: `:root[data-theme=X]` and `.dark[data-theme=X]` have
 * IDENTICAL specificity (0,2,0), so in the browser only document order
 * separates them — the dark block must physically follow the light one. This
 * function assumes that ordering; `assertBlockOrder` below verifies it, because
 * a file where they are swapped renders light tokens in dark mode while every
 * contrast assertion still passes.
 */
export function buildScopes(css) {
  const THEME = merge(...blocks(css, (s) => s.startsWith('@theme')));
  const LIGHT = merge(...blocks(css, (s) => s === ':root'));
  const DARK = merge(...blocks(css, (s) => s === '.dark'));

  const scopes = {
    'parchment:light': { decls: { ...LIGHT }, theme: THEME },
    'parchment:dark': { decls: { ...LIGHT, ...DARK }, theme: THEME },
  };

  for (const family of discoverFamilies(css)) {
    const L = merge(...blocks(css, (s) => s === `:root[data-theme='${family}']`));
    const D = merge(...blocks(css, (s) => s === `.dark[data-theme='${family}']`));
    scopes[`${family}:light`] = { decls: { ...LIGHT, ...L }, theme: THEME };
    scopes[`${family}:dark`] = { decls: { ...LIGHT, ...DARK, ...L, ...D }, theme: THEME };
  }
  return scopes;
}

/**
 * Verify that each family's dark block physically follows its light block.
 * Equal specificity means source order decides; getting this wrong renders
 * light tokens in dark mode with no assertion failing.
 */
export function assertBlockOrder(css) {
  const problems = [];
  for (const family of discoverFamilies(css)) {
    const li = css.indexOf(`:root[data-theme='${family}'] {`);
    const di = css.indexOf(`.dark[data-theme='${family}'] {`);
    if (li === -1 || di === -1) continue;
    if (di < li) {
      problems.push(
        `${family}: .dark[data-theme='${family}'] appears BEFORE :root[data-theme='${family}']. ` +
          `They have equal specificity (0,2,0), so dark mode will render light tokens.`
      );
    }
  }
  return problems;
}

/** Resolve a token through its var() chain to a literal. Returns null if unset. */
export function resolveRaw(name, scope) {
  let v = scope.decls[name] ?? scope.theme[name];
  for (let i = 0; i < 12 && v && v.startsWith('var('); i++) {
    const inner = v.slice(4, v.indexOf(')')).trim();
    v = scope.decls[inner] ?? scope.theme[inner];
  }
  return v ? v.trim() : null;
}

/** Resolve to a 6-digit hex, or null if the value is not one. */
export function resolveHex(name, scope) {
  const v = resolveRaw(name, scope);
  return v && /^#[0-9a-f]{6}$/i.test(v) ? v.toLowerCase() : null;
}

/**
 * The hex a translucent fill actually paints when composited over `groundHex`.
 *
 * Tailwind's `/12`-style opacity modifiers (`bg-background-accent/12`, the
 * `Badge` accent tone) never reach the cascade as a token, so a guard that only
 * resolves opaque token pairs cannot see them — and what the eye reads is the
 * blend, not the token. Issue #65's reference chip shipped accent ink on a 12%
 * accent fill that measured 3.08:1 inside a user bubble with every existing
 * assertion green.
 *
 * `alpha` is the fill's opacity in 0..1. Both inputs must be 6-digit hex, for
 * the same fail-closed reason as `luminance`.
 */
export function blend(fillHex, alpha, groundHex) {
  const channels = (h) => {
    if (typeof h !== 'string' || !/^#[0-9a-fA-F]{6}$/.test(h.trim())) {
      throw new TypeError(`blend() needs a 6-digit hex, got ${JSON.stringify(h)}`);
    }
    return [1, 3, 5].map((i) => parseInt(h.slice(i, i + 2), 16));
  };
  const [f, g] = [channels(fillHex), channels(groundHex)];
  return `#${f
    .map((v, i) => Math.round(v * alpha + g[i] * (1 - alpha)))
    .map((v) => v.toString(16).padStart(2, '0'))
    .join('')}`;
}

/* ── WCAG maths — the single implementation ── */

export const luminance = (h) => {
  // FAIL CLOSED. This slices fixed hex offsets, so any other notation silently
  // yields NaN — and `if (ratio < floor)` is FALSE for NaN, which means a
  // non-hex colour would quietly exempt itself from every contrast check that
  // uses a bare comparison. The generator's terminal and splash assertions had
  // exactly that hole: `mark.navy: 'rgb(27, 27, 25)'` emitted an invisible
  // 1.09:1 boot mark with every gate green. Throwing here closes it for every
  // caller at once rather than per-assertion.
  if (typeof h !== 'string' || !/^#[0-9a-fA-F]{6}$/.test(h.trim())) {
    throw new TypeError(
      `luminance() needs a 6-digit hex, got ${JSON.stringify(h)}. ` +
        `Non-hex values cannot be contrast-checked — give the token a hex, or ` +
        `exclude it from the check explicitly.`
    );
  }
  const c = [1, 3, 5]
    .map((i) => parseInt(h.slice(i, i + 2), 16) / 255)
    .map((v) => (v <= 0.03928 ? v / 12.92 : Math.pow((v + 0.055) / 1.055, 2.4)));
  return 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
};

export const contrast = (a, b) => {
  const [x, y] = [luminance(a), luminance(b)];
  return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05);
};

/* ── perceptual difference and colour-vision simulation ──
   Added for the knowledge-graph palette (ui-spec.md §5.3.2, §6.4). ONE
   implementation each, here beside `contrast` and `blend`, for the reason this
   module exists at all: the previous generation of guards each reimplemented
   the WCAG maths and one of them came to verify a syntax palette against a
   surface the app never painted. */

const D65 = [0.95047, 1, 1.08883];
const LAB_E = 216 / 24389;
const LAB_K = 24389 / 27;
const RAD = Math.PI / 180;
const DEG = 180 / Math.PI;

/** sRGB transfer function and its inverse, on 0..1 channels. */
const toLinear = (v) => (v <= 0.04045 ? v / 12.92 : Math.pow((v + 0.055) / 1.055, 2.4));
const toEncoded = (v) => (v <= 0.0031308 ? 12.92 * v : 1.055 * Math.pow(v, 1 / 2.4) - 0.055);

/** 0..1 linear-light channels for a 6-digit hex. Fails closed, like `luminance`. */
function linearChannels(hex) {
  if (typeof hex !== 'string' || !/^#[0-9a-fA-F]{6}$/.test(hex.trim())) {
    throw new TypeError(`expected a 6-digit hex, got ${JSON.stringify(hex)}`);
  }
  return [1, 3, 5].map((i) => toLinear(parseInt(hex.slice(i, i + 2), 16) / 255));
}

const hexFromLinear = (rgb) =>
  '#' +
  rgb
    .map((v) => {
      const enc = toEncoded(Math.min(1, Math.max(0, v)));
      return Math.round(Math.min(1, Math.max(0, enc)) * 255)
        .toString(16)
        .padStart(2, '0');
    })
    .join('');

/** CIELAB (D65, 2°) for a 6-digit hex. */
export function hexToLab(hex) {
  const [r, g, b] = linearChannels(hex);
  const xyz = [
    0.4124564 * r + 0.3575761 * g + 0.1804375 * b,
    0.2126729 * r + 0.7151522 * g + 0.072175 * b,
    0.0193339 * r + 0.119192 * g + 0.9503041 * b,
  ];
  const [fx, fy, fz] = xyz.map((v, i) => {
    const t = v / D65[i];
    return t > LAB_E ? Math.cbrt(t) : (LAB_K * t + 16) / 116;
  });
  return [116 * fy - 16, 500 * (fx - fy), 200 * (fy - fz)];
}

/**
 * CIEDE2000 between two CIELAB triples — Sharma, Wu & Dalal (2005).
 *
 * ⚠ A WRONG ΔE00 MAKES EVERY AUDIT THAT USES IT PASS VACUOUSLY, and this is
 * the easiest function in the file to get subtly wrong: the mean-hue
 * discontinuity at ±180° and the R_T rotation term are both easy to
 * mis-transcribe, and neither shows up on well-separated colours — which is
 * most of a palette. It therefore ships with the published Sharma–Wu–Dalal
 * 34-pair reference table checked in at `__fixtures__/ciede2000-sharma.json`,
 * asserted to 4 decimal places by `src/styles/graphPalette.test.ts`. That table
 * is the correctness gate; palette figures are regression vectors against one
 * palette and prove nothing about this implementation on their own.
 */
export function deltaE00Lab(labA, labB) {
  const [L1, a1, b1] = labA;
  const [L2, a2, b2] = labB;
  const C1 = Math.hypot(a1, b1);
  const C2 = Math.hypot(a2, b2);
  const Cbar7 = Math.pow((C1 + C2) / 2, 7);
  const G = 0.5 * (1 - Math.sqrt(Cbar7 / (Cbar7 + Math.pow(25, 7))));
  const ap1 = (1 + G) * a1;
  const ap2 = (1 + G) * a2;
  const Cp1 = Math.hypot(ap1, b1);
  const Cp2 = Math.hypot(ap2, b2);
  // atan2(0, 0) is 0 by definition here: a neutral has no hue, and letting the
  // platform decide would make the result depend on the sign of a zero.
  const hueOf = (b, a) => {
    if (b === 0 && a === 0) return 0;
    const h = Math.atan2(b, a) * DEG;
    return h < 0 ? h + 360 : h;
  };
  const hp1 = hueOf(b1, ap1);
  const hp2 = hueOf(b2, ap2);

  const dLp = L2 - L1;
  const dCp = Cp2 - Cp1;
  let dhp = 0;
  if (Cp1 * Cp2 !== 0) {
    dhp = hp2 - hp1;
    if (dhp > 180) dhp -= 360;
    else if (dhp < -180) dhp += 360;
  }
  const dHp = 2 * Math.sqrt(Cp1 * Cp2) * Math.sin((dhp / 2) * RAD);

  const Lbp = (L1 + L2) / 2;
  const Cbp = (Cp1 + Cp2) / 2;
  let hbp;
  if (Cp1 * Cp2 === 0) {
    hbp = hp1 + hp2;
  } else if (Math.abs(hp1 - hp2) <= 180) {
    hbp = (hp1 + hp2) / 2;
  } else if (hp1 + hp2 < 360) {
    hbp = (hp1 + hp2 + 360) / 2;
  } else {
    hbp = (hp1 + hp2 - 360) / 2;
  }

  const T =
    1 -
    0.17 * Math.cos((hbp - 30) * RAD) +
    0.24 * Math.cos(2 * hbp * RAD) +
    0.32 * Math.cos((3 * hbp + 6) * RAD) -
    0.2 * Math.cos((4 * hbp - 63) * RAD);
  const dTheta = 30 * Math.exp(-Math.pow((hbp - 275) / 25, 2));
  const Cbp7 = Math.pow(Cbp, 7);
  const RC = 2 * Math.sqrt(Cbp7 / (Cbp7 + Math.pow(25, 7)));
  const SL = 1 + (0.015 * Math.pow(Lbp - 50, 2)) / Math.sqrt(20 + Math.pow(Lbp - 50, 2));
  const SC = 1 + 0.045 * Cbp;
  const SH = 1 + 0.015 * Cbp * T;
  const RT = -Math.sin(2 * dTheta * RAD) * RC;

  return Math.sqrt(
    Math.pow(dLp / SL, 2) +
      Math.pow(dCp / SC, 2) +
      Math.pow(dHp / SH, 2) +
      RT * (dCp / SC) * (dHp / SH)
  );
}

/** CIEDE2000 between two 6-digit hexes. */
export const deltaE00 = (a, b) => deltaE00Lab(hexToLab(a), hexToLab(b));

/**
 * Dichromacy simulation — Viénot, Brettel & Mollon (1999), applied in
 * linear-light sRGB via the LMS transform.
 *
 * Stating the model is part of the specification: a guard whose simulation is
 * unstated is not reproducible, and the two common variants (this one, and the
 * same matrices applied to gamma-ENCODED values) disagree by enough to move a
 * measured minimum. `simulateCvd` is the one that ui-spec.md §5.3.2's figures
 * were measured with.
 */
const RGB_TO_LMS = [
  [17.8824, 43.5161, 4.11935],
  [3.45565, 27.1554, 3.86714],
  [0.0299566, 0.184309, 1.46709],
];
const LMS_TO_RGB = [
  [0.080944, -0.130504, 0.116721],
  [-0.0102485, 0.0540194, -0.113615],
  [-0.000365294, -0.00412163, 0.693513],
];
const DICHROMAT = {
  protan: [
    [0, 2.02344, -2.52581],
    [0, 1, 0],
    [0, 0, 1],
  ],
  deutan: [
    [1, 0, 0],
    [0.494207, 0, 1.24827],
    [0, 0, 1],
  ],
  tritan: [
    [1, 0, 0],
    [0, 1, 0],
    [-0.395913, 0.801109, 0],
  ],
};
const apply = (m, v) => m.map((row) => row[0] * v[0] + row[1] * v[1] + row[2] * v[2]);

export function simulateCvd(hex, kind) {
  const matrix = DICHROMAT[kind];
  if (!matrix) {
    throw new TypeError(`simulateCvd: unknown deficiency ${JSON.stringify(kind)}`);
  }
  return hexFromLinear(apply(LMS_TO_RGB, apply(matrix, apply(RGB_TO_LMS, linearChannels(hex)))));
}
