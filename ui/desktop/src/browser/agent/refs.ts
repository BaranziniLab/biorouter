/**
 * Element references for one browser tab (BB-12).
 *
 * The rules, each from agent-browser's ref model:
 *
 * - **Storage.** A reference names a node by its `backendDOMNodeId` in one frame, with the role,
 *   the name and whether it was unique in the snapshot that made it. Never a CSS path.
 * - **Lifetime.** A reference stays valid while its node is alive in the same document; a node
 *   keeps its reference across later snapshots.
 * - **Numbers are never reused.** One counter per tab that never resets, even across navigation,
 *   so an old `@e5` can never land on a new node.
 * - **Invalidation.** A main document navigation invalidates every reference in the tab; a frame
 *   navigation invalidates that frame's; discarding the tab invalidates all of them. An
 *   invalidated reference fails with the reason and what to do.
 *
 * Pure: no Electron, so Vitest exercises it directly.
 */

export type RefTarget = {
  ref: string;
  backendNodeId: number;
  /** `main`, or the frame id of a child frame. */
  frameKey: string;
  role: string;
  name: string;
  /** Whether (role, name) was unique among the snapshot's references. */
  unique: boolean;
};

type Retired = { url: string; reason: 'navigated' | 'frame' | 'discarded' };

/** How many retired references keep their reason; older ones get the general sentence. */
const MAX_RETIRED = 2_000;

export class RefStore {
  private counter = 0;
  private readonly live = new Map<string, RefTarget>();
  private readonly byNode = new Map<string, string>();
  private readonly retired = new Map<string, Retired>();

  private static nodeKey(frameKey: string, backendNodeId: number): string {
    return `${frameKey}|${backendNodeId}`;
  }

  /** The reference for a node, keeping the one it already has. */
  assign(target: Omit<RefTarget, 'ref'>): string {
    const key = RefStore.nodeKey(target.frameKey, target.backendNodeId);
    const existing = this.byNode.get(key);
    if (existing) {
      // Refresh what the latest snapshot said about it: the name and its uniqueness can change.
      this.live.set(existing, { ...target, ref: existing });
      return existing;
    }
    this.counter += 1;
    const ref = `@e${this.counter}`;
    this.live.set(ref, { ...target, ref });
    this.byNode.set(key, ref);
    return ref;
  }

  get(ref: string): RefTarget | null {
    return this.live.get(normalizeRef(ref)) ?? null;
  }

  /** The live reference a node already has, if any. */
  find(frameKey: string, backendNodeId: number): RefTarget | null {
    const ref = this.byNode.get(RefStore.nodeKey(frameKey, backendNodeId));
    return ref ? (this.live.get(ref) ?? null) : null;
  }

  /** The last number handed out; tests and the counter-never-resets rule read it. */
  get lastNumber(): number {
    return this.counter;
  }

  get size(): number {
    return this.live.size;
  }

  /** A node is gone for good (its document changed under it): drop its reference. */
  forget(ref: string, url: string): void {
    const target = this.live.get(ref);
    if (!target) return;
    this.live.delete(ref);
    this.byNode.delete(RefStore.nodeKey(target.frameKey, target.backendNodeId));
    this.retire(ref, { url, reason: 'navigated' });
  }

  /** The main document navigated: every reference in the tab goes. */
  invalidateAll(url: string, reason: Retired['reason'] = 'navigated'): boolean {
    const had = this.live.size > 0;
    for (const ref of this.live.keys()) this.retire(ref, { url, reason });
    this.live.clear();
    this.byNode.clear();
    return had;
  }

  /** One frame navigated: only its references go. */
  invalidateFrame(frameKey: string, url: string): boolean {
    let had = false;
    for (const [ref, target] of [...this.live.entries()]) {
      if (target.frameKey !== frameKey) continue;
      had = true;
      this.live.delete(ref);
      this.byNode.delete(RefStore.nodeKey(target.frameKey, target.backendNodeId));
      this.retire(ref, { url, reason: 'frame' });
    }
    return had;
  }

  /** Why `ref` does not resolve, and what to do. */
  staleReason(ref: string, currentUrl: string): string {
    const normalized = normalizeRef(ref);
    const retired = this.retired.get(normalized);
    const where = currentUrl ? ` the tab is now at ${currentUrl}.` : '';
    if (retired?.reason === 'frame') {
      return `${normalized} was in a frame that has since navigated;${where} Take a new browser_snapshot.`;
    }
    if (retired?.reason === 'discarded') {
      return `${normalized} is from a tab that was reloaded;${where} Take a new browser_snapshot.`;
    }
    if (retired) {
      return `${normalized} is from an earlier page;${where} Take a new browser_snapshot.`;
    }
    const number = Number(normalized.slice(2));
    if (Number.isFinite(number) && number > 0 && number <= this.counter) {
      return `${normalized} is from an earlier page;${where} Take a new browser_snapshot.`;
    }
    return `${normalized} is not a reference from this tab. Take a browser_snapshot and use one of its @e references.`;
  }

  private retire(ref: string, why: Retired): void {
    this.retired.set(ref, why);
    if (this.retired.size > MAX_RETIRED) {
      const oldest = this.retired.keys().next().value;
      if (oldest !== undefined) this.retired.delete(oldest);
    }
  }
}

/** `e5`, `@e5` and ` @e5 ` are the same reference. */
export function normalizeRef(ref: string): string {
  const trimmed = ref.trim();
  return trimmed.startsWith('@') ? trimmed : `@${trimmed}`;
}

/**
 * The fallback when a node is gone but the document is the same (BB-12): only a reference that
 * was unique in its snapshot may move, and only to a single node with the same role and name.
 * Two or more matches fail closed.
 */
export function fallbackMatch(
  target: RefTarget,
  candidates: Array<{ backendNodeId: number; role: string; name: string }>
): { ok: true; backendNodeId: number } | { ok: false; reason: string } {
  if (!target.unique) {
    return {
      ok: false,
      reason: `${target.ref} no longer exists, and it was not unique when it was taken. Take a new browser_snapshot.`,
    };
  }
  const matches = candidates.filter(
    (candidate) => candidate.role === target.role && candidate.name === target.name
  );
  if (matches.length === 1) return { ok: true, backendNodeId: matches[0].backendNodeId };
  if (matches.length === 0) {
    return {
      ok: false,
      reason: `${target.ref} no longer exists. Take a new browser_snapshot.`,
    };
  }
  return {
    ok: false,
    reason: `${target.ref} no longer exists and ${matches.length} elements now match [${target.role}] "${target.name}". Take a new browser_snapshot.`,
  };
}
