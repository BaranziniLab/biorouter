import {
  createContext,
  useContext,
  useEffect,
  useState,
  useSyncExternalStore,
  type ReactNode,
} from 'react';
import { shortTime } from '../timeline/timelineTime';

/**
 * The files the loaded messages of one channel carry, as their cards learned them (Q3-13).
 *
 * A message names its files by ID only; a card learns the file's name, checksum and state from
 * `blob.status`. Two things need to know about the OTHER cards on screen, and nothing else has
 * that view:
 *
 * - a card whose name another loaded card shares adds its post time to its controls' names and
 *   its meta ("Save counts.csv, 6:54 PM", "100 bytes · 6:54 PM"), so two same-named files are
 *   never two identical cards with identical buttons;
 * - the composer says when a file in the draft is already in the channel — the same name, or the
 *   same contents once its checksum is known — so sharing the same file twice is a choice.
 *
 * Every mounted card registers what it learned, keyed by its file, and takes it back on unmount;
 * the same file shown twice (the timeline and the Files tab) counts once. The layout provides one
 * index for the channel view. Without a provider nothing registers and nothing is compared.
 *
 * Display only: this names what is on screen and decides nothing. The broker and the daemon still
 * decide every upload and every post.
 */

export interface IndexedAttachment {
  name: string;
  /** Hex SHA-256, or '' when the card has not learned it. */
  sha256: string;
  /** The upload finished: the file can be read from the channel. */
  complete: boolean;
  /** When the message carrying it was posted (Unix milliseconds), when known. */
  postedAt: number | null;
}

interface Entry {
  value: IndexedAttachment;
  refs: number;
}

const MONTH_DAY = new Intl.DateTimeFormat('en-US', { month: 'short', day: 'numeric' });

/**
 * When a file was shared, as short as tells it apart: "6:54 PM" today, "Sep 22, 6:54 PM" on any
 * other day. '' when the time is not known.
 */
export function postedLabel(postedAt: number | null, now: Date = new Date()): string {
  if (postedAt === null || !Number.isFinite(postedAt)) return '';
  const date = new Date(postedAt);
  if (Number.isNaN(date.getTime())) return '';
  const today =
    date.getFullYear() === now.getFullYear() &&
    date.getMonth() === now.getMonth() &&
    date.getDate() === now.getDate();
  return today ? shortTime(date) : `${MONTH_DAY.format(date)}, ${shortTime(date)}`;
}

export class AttachmentIndex {
  private entries = new Map<string, Entry>();
  private listeners = new Set<() => void>();
  private version = 0;

  /** Record what a card learned about `blobId`. Returns the matching take-back. */
  register(blobId: string, value: IndexedAttachment): () => void {
    const entry = this.entries.get(blobId);
    if (entry) {
      entry.refs += 1;
      entry.value = value;
    } else {
      this.entries.set(blobId, { value, refs: 1 });
    }
    this.changed();
    let done = false;
    return () => {
      if (done) return;
      done = true;
      const current = this.entries.get(blobId);
      if (!current) return;
      current.refs -= 1;
      if (current.refs <= 0) this.entries.delete(blobId);
      this.changed();
    };
  }

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };

  /** Bumped on every change, for a reader that compares the whole index. */
  getVersion = (): number => this.version;

  /**
   * What tells this card apart from another loaded card with the same name: its post time, and
   * ", 2 of 2" after it when even the times read the same. '' while its name is its own.
   */
  which(blobId: string, name: string, postedAt: number | null): string {
    const namesakes = [...this.entries.entries()].filter(
      ([id, entry]) => id !== blobId && entry.value.name === name
    );
    if (namesakes.length === 0) return '';
    const label = postedLabel(postedAt);
    if (!label) return '';
    const sameLabel = [
      { id: blobId, postedAt },
      ...namesakes
        .filter(([, entry]) => postedLabel(entry.value.postedAt) === label)
        .map(([id, entry]) => ({ id, postedAt: entry.value.postedAt })),
    ].sort((a, b) => (a.postedAt ?? 0) - (b.postedAt ?? 0) || a.id.localeCompare(b.id));
    if (sameLabel.length < 2) return label;
    const index = sameLabel.findIndex((item) => item.id === blobId) + 1;
    return `${label}, ${index} of ${sameLabel.length}`;
  }

  /**
   * A finished file in the channel that a draft file looks like: the same name, or the same
   * contents when both checksums are known. The newest such, or null. `excludeId` is the draft
   * file itself, never its own match.
   */
  existing(name: string, sha256: string, excludeId: string): IndexedAttachment | null {
    let found: IndexedAttachment | null = null;
    for (const [id, { value }] of this.entries) {
      if (id === excludeId || !value.complete) continue;
      const sameContents = Boolean(sha256) && Boolean(value.sha256) && value.sha256 === sha256;
      if (value.name !== name && !sameContents) continue;
      if (!found || (value.postedAt ?? 0) > (found.postedAt ?? 0)) found = value;
    }
    return found;
  }

  private changed() {
    this.version += 1;
    for (const listener of this.listeners) listener();
  }
}

const AttachmentIndexContext = createContext<AttachmentIndex | null>(null);

/** One index for a channel view: its timeline, its Files tab and its composer. */
export function AttachmentIndexProvider({ children }: { children: ReactNode }) {
  const [index] = useState(() => new AttachmentIndex());
  return (
    <AttachmentIndexContext.Provider value={index}>{children}</AttachmentIndexContext.Provider>
  );
}

/** The channel view's index, or null outside one. */
export function useAttachmentIndex(): AttachmentIndex | null {
  return useContext(AttachmentIndexContext);
}

const NO_SUBSCRIPTION = () => () => {};

/** Register what this card learned, while mounted and while it knows it. */
export function useRegisterAttachment(blobId: string, value: IndexedAttachment | null): void {
  const index = useAttachmentIndex();
  const name = value?.name;
  const sha256 = value?.sha256 ?? '';
  const complete = value?.complete ?? false;
  const postedAt = value?.postedAt ?? null;
  useEffect(() => {
    if (!index || name === undefined) return;
    return index.register(blobId, { name, sha256, complete, postedAt });
  }, [index, blobId, name, sha256, complete, postedAt]);
}

/** What tells this card apart from a same-named one (see {@link AttachmentIndex.which}). */
export function useAttachmentWhich(blobId: string, name: string, postedAt: number | null): string {
  const index = useAttachmentIndex();
  return useSyncExternalStore(index?.subscribe ?? NO_SUBSCRIPTION, () =>
    index ? index.which(blobId, name, postedAt) : ''
  );
}

/** The index and its version, for a reader that compares a draft against every loaded file. */
export function useAttachmentIndexVersion(): [AttachmentIndex | null, number] {
  const index = useAttachmentIndex();
  const version = useSyncExternalStore(index?.subscribe ?? NO_SUBSCRIPTION, () =>
    index ? index.getVersion() : 0
  );
  return [index, version];
}
