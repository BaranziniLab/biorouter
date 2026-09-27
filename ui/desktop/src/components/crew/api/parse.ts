// Small, dependency-free readers for daemon JSON. A route's helper validates what it returns, so a
// component never renders an unchecked value.

export function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
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
