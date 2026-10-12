type DaemonKeys = { secretKey: string; userActionKey: string };

export function attachedDaemonKey(
  daemons: ReadonlyMap<number, DaemonKeys>,
  windowId: number | undefined,
  key: keyof DaemonKeys,
  fallback: () => string
): string {
  const daemon = windowId === undefined ? undefined : daemons.get(windowId);
  return daemon ? daemon[key] : fallback();
}
