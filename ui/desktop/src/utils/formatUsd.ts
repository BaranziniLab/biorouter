/**
 * The one way every usage surface prints dollars: the composer's cost and its card, the per-model
 * rows, the limit controls, Settings > App > Usage. The daemon's limit notices print them the same
 * way (`format_usd` and `format_usd_pair` in `crates/biorouter/src/agents/usage_limit.rs`); both
 * run every case in `formatUsdCases.json`, so a figure on the composer and the same figure in a
 * notice never differ.
 *
 * From $1 up: two decimals, the whole dollars grouped ("$1,250.00"). Below $1: three significant
 * digits but never fewer than three decimals, a tenth of a cent, with trailing zeros past the
 * cents dropped ("$0.18", "$0.177", "$0.0085"), so a sub-cent spend never reads "<$0.01" and a
 * $0.0085 chat never reads "$0.01" in one place and "<$0.01" in another.
 */

const MAX_DECIMALS = 6;

function decimalsFor(amount: number): number {
  if (!(amount > 0) || amount >= 1) return 2;
  const firstSignificant = Math.floor(-Math.log10(amount)) + 1;
  return Math.min(Math.max(firstSignificant + 2, 3), MAX_DECIMALS);
}

function groupThousands(whole: string): string {
  return whole.replace(/\B(?=(\d{3})+(?!\d))/g, ',');
}

function render(amount: number, decimals: number, trim: boolean): string {
  const fixed = Math.max(0, amount).toFixed(decimals);
  const [whole, fraction = ''] = fixed.split('.');
  let digits = fraction;
  if (trim) {
    while (digits.length > 2 && digits.endsWith('0')) digits = digits.slice(0, -1);
  }
  return digits ? `$${groupThousands(whole)}.${digits}` : `$${groupThousands(whole)}`;
}

/** "$5.00", "$1,250.00", "$0.18", "$0.177", "$0.0085"; "$0.00" for nothing or a bad number. */
export function formatUsd(amount: number): string {
  if (!Number.isFinite(amount) || amount <= 0) return '$0.00';
  if (amount < 10 ** -MAX_DECIMALS) return `<$${(10 ** -MAX_DECIMALS).toFixed(MAX_DECIMALS)}`;
  return render(amount, decimalsFor(amount), true);
}

/**
 * Two amounts read side by side, such as what was used and its limit: each as {@link formatUsd}
 * prints it, unless two different amounts would read alike ("$0.18 of $0.18" for $0.17996), in
 * which case both take the fewest decimals that tell them apart.
 */
export function formatUsdPair(a: number, b: number): [string, string] {
  const pair: [string, string] = [formatUsd(a), formatUsd(b)];
  if (pair[0] !== pair[1] || a === b || !Number.isFinite(a) || !Number.isFinite(b)) return pair;
  for (let decimals = 2; decimals <= MAX_DECIMALS; decimals += 1) {
    const x = render(a, decimals, false);
    const y = render(b, decimals, false);
    if (x !== y) return [x, y];
  }
  return pair;
}
