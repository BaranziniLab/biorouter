/**
 * The US/British variant pairs this repository can plausibly disagree about,
 * with every inflection each pair takes.
 *
 * A pair is `[american, british]` and the guard rejects the LEFT one — see
 * `uiCopySpelling.test.ts` for the convention and why it was chosen.
 *
 * Pairs that are a matter of STYLE rather than spelling are deliberately absent
 * — `toward`/`towards`, `while`/`whilst`, `among`/`amongst` are all current in
 * British English, so rejecting one of each would be enforcing a preference the
 * convention does not hold.
 */
export type VariantPair = readonly [american: string, british: string];

/**
 * `-ize`/`-ise` stems. Each yields the verb, its three inflections and — unless
 * listed in `NO_ATION_NOUN` — its `-ization`/`-isation` noun.
 *
 * The `-ize` spelling is not in fact wrong in British English (Oxford uses it),
 * which is exactly why the stem list matters: this repository's own prose is
 * consistently `-ise`, and a convention is a choice between two defensible
 * forms, not a correction of an error.
 */
const IZE_STEMS = [
  'analy',
  'apologi',
  'authori',
  'capitali',
  'categori',
  'centrali',
  'customi',
  'emphasi',
  'finali',
  'formali',
  'generali',
  'initiali',
  'maximi',
  'minimi',
  'moderni',
  'normali',
  'optimi',
  'organi',
  'personali',
  'prioriti',
  'randomi',
  'reali',
  'recogni',
  'saniti',
  'seriali',
  'speciali',
  'standardi',
  'summari',
  'synchroni',
  'tokeni',
  'utili',
  'visuali',
] as const;

/**
 * Stems whose noun is not formed by adding `-ation`: the noun of `analyse` is
 * `analysis`, of `emphasise` `emphasis`, of `recognise` `recognition`, and of
 * `apologise` `apology`. Generating `analyzation` would put a non-word in the
 * guard, where it could only ever produce a confusing failure message.
 */
const NO_ATION_NOUN = new Set(['analy', 'apologi', 'emphasi', 'recogni']);

function izePairs(): VariantPair[] {
  const pairs: VariantPair[] = [];
  for (const stem of IZE_STEMS) {
    // `-er` is the agent noun, and it is the form that carries the one product
    // name this convention cannot touch: **Auto Visualiser**. Generating it is
    // deliberate — the guard must be able to see `visualiser`, so that the
    // allow-list exempting the product name is doing real work rather than
    // describing a case that could never arise.
    for (const suffix of ['e', 'es', 'ed', 'ing', 'er', 'ers']) {
      pairs.push([`${stem}z${suffix}`, `${stem}s${suffix}`]);
    }
    if (!NO_ATION_NOUN.has(stem)) {
      pairs.push([`${stem}zation`, `${stem}sation`]);
      pairs.push([`${stem}zations`, `${stem}sations`]);
    }
  }
  return pairs;
}

const OTHER_PAIRS: VariantPair[] = [
  // -or / -our
  ['behavior', 'behaviour'],
  ['behaviors', 'behaviours'],
  ['behavioral', 'behavioural'],
  ['color', 'colour'],
  ['colors', 'colours'],
  ['colored', 'coloured'],
  ['coloring', 'colouring'],
  ['colorful', 'colourful'],
  ['favorite', 'favourite'],
  ['favorites', 'favourites'],
  ['favor', 'favour'],
  ['favored', 'favoured'],
  ['flavor', 'flavour'],
  ['flavors', 'flavours'],
  ['honor', 'honour'],
  ['honors', 'honours'],
  ['honored', 'honoured'],
  ['labor', 'labour'],
  ['neighbor', 'neighbour'],
  ['neighbors', 'neighbours'],
  ['neighboring', 'neighbouring'],
  // -er / -re
  ['center', 'centre'],
  ['centers', 'centres'],
  ['centered', 'centred'],
  ['fiber', 'fibre'],
  ['theater', 'theatre'],
  // `meter`/`metre` is NOT a pair. British English spells the measuring DEVICE
  // `meter` and only the unit of length `metre`, so "token meter" in
  // `ResetPanel` is correct under either convention and listing the pair would
  // report a dialect disagreement that does not exist.
  // -se / -ce
  ['defense', 'defence'],
  ['defenses', 'defences'],
  ['license', 'licence'],
  ['licenses', 'licences'],
  ['offense', 'offence'],
  // single / doubled consonant
  ['canceled', 'cancelled'],
  ['canceling', 'cancelling'],
  ['cancelation', 'cancellation'],
  ['enrollment', 'enrolment'],
  ['fulfill', 'fulfil'],
  ['fulfills', 'fulfils'],
  ['fulfillment', 'fulfilment'],
  ['installment', 'instalment'],
  ['labeled', 'labelled'],
  ['labeling', 'labelling'],
  ['modeled', 'modelled'],
  ['modeling', 'modelling'],
  ['signaled', 'signalled'],
  ['signaling', 'signalling'],
  ['skillful', 'skilful'],
  ['traveled', 'travelled'],
  ['traveling', 'travelling'],
  // one-offs
  ['aging', 'ageing'],
  ['analog', 'analogue'],
  ['artifact', 'artefact'],
  ['artifacts', 'artefacts'],
  ['catalog', 'catalogue'],
  ['catalogs', 'catalogues'],
  ['dialog', 'dialogue'],
  ['dialogs', 'dialogues'],
  ['gray', 'grey'],
  ['grayed', 'greyed'],
  ['judgment', 'judgement'],
  ['judgments', 'judgements'],
  ['program', 'programme'],
  ['programs', 'programmes'],
];

export const VARIANT_PAIRS: readonly VariantPair[] = [...izePairs(), ...OTHER_PAIRS];

/**
 * The convention, in one place: user-visible product copy is **American
 * English**. Decided 2026-09-09 from a measurement of every user-visible string
 * in the product — see `uiCopySpelling.test.ts` for the counts and the
 * trade-off.
 *
 * Flipping the convention is deliberately a one-word edit here: `ACCEPTED`
 * names the column that is kept and `REJECTED_FORMS` is derived from the other,
 * so nothing downstream encodes which dialect won.
 */
export const ACCEPTED_CONVENTION: 'american' | 'british' = 'american';

/** Every variant the convention rejects, mapped to what to write instead. */
export const REJECTED_FORMS: ReadonlyMap<string, string> = new Map(
  VARIANT_PAIRS.map(([american, british]) =>
    ACCEPTED_CONVENTION === 'american' ? [british, american] : [american, british]
  )
);

/** Whole-word, case-insensitive occurrences of `word` in `text`. */
export function variantMatches(text: string, word: string): number {
  const pattern = new RegExp(`(?<![A-Za-z])${word}(?![A-Za-z])`, 'gi');
  return (text.match(pattern) ?? []).length;
}
