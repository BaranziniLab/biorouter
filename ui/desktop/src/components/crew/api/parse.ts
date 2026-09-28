// Small, dependency-free readers for daemon JSON. A route's helper validates what it returns, so a
// component never renders an unchecked value.

/** The keys of every member of a union: a tagged answer's fields, whichever tag it carries. */
type KeysOf<T> = T extends unknown ? keyof T : never;

/**
 * A daemon answer as a reader meets it: each field the generated type `T` declares, of unknown
 * value. A reader validates every value, because a daemon of another version (or a workspace's
 * words inside one) can send anything; but it can only name a field the daemon's OpenAPI spec
 * declares, so a field the daemon renames or drops fails to compile here instead of reading as
 * absent (CROSSCUT-6).
 */
export type Wire<T> = { readonly [K in KeysOf<T>]?: unknown };

/**
 * `T` with the fields `K` optional, for a field an older daemon did not send. `K` must be a field
 * of `T`, so the name stays checked against the generated type.
 */
export type Loosen<T, K extends keyof T> = Omit<T, K> & Partial<Pick<T, K>>;

export function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

/** `value` read as the answer `T` is declared to be, when it is an object at all. */
export function wireOf<T>(value: unknown): Wire<T> | undefined {
  return isRecord(value) ? (value as Wire<T>) : undefined;
}

/** A string with visible content, else `undefined`. */
export function optionalText(value: unknown): string | undefined {
  return typeof value === 'string' && value.trim().length > 0 ? value : undefined;
}

/** A finite number, else `undefined`. */
export function optionalNumber(value: unknown): number | undefined {
  return typeof value === 'number' && Number.isFinite(value) ? value : undefined;
}

/** A string with visible content, `null` when the daemon sent null, else `undefined`. */
export function nullableText(value: unknown): string | null | undefined {
  return value === null ? null : optionalText(value);
}

/** A finite number, `null` when the daemon sent null, else `undefined`. */
export function nullableNumber(value: unknown): number | null | undefined {
  return value === null ? null : optionalNumber(value);
}

export function stringArray(value: unknown): string[] | undefined {
  return Array.isArray(value) && value.every((item) => typeof item === 'string')
    ? [...value]
    : undefined;
}
