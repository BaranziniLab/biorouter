/**
 * Copy text for a `⋯` menu's "Copy …" item. A menu closes as it acts, so there is nowhere to say
 * "Copied" and, by the copy deck's rule, copying never toasts; a refusal is swallowed for the same
 * reason. Anything a person must be SURE they copied is a `CopyField`, which confirms in place.
 */
export async function copyText(text: string): Promise<boolean> {
  try {
    if (!navigator.clipboard?.writeText) return false;
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    return false;
  }
}
