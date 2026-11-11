/**
 * The words of the external-folder approval card (`ToolCallConfirmation` with a
 * `folderAccess` preview), kept apart from the card so they can be tested alone.
 *
 * R2-1 (live QA, 2026-11-10): the card opened with the daemon's prompt, "External
 * folder access Allow read-only access to /private/tmp/notes for
 * developer__text_editor? …", then asked the same question again below it, never
 * said which subagent was asking, and named the folder by its resolved
 * `/private/...` spelling. Now its one line says who asks and for which folder,
 * written as the person would write it (`~` for home, the path the call asked
 * for), with no tool id; the exact paths and the grant's terms sit under Details.
 */
import { createContext, useContext } from 'react';
import type { ToolPreview } from '../api';
import { homeFolder, tildePath } from '../utils/folderPath';

/**
 * Who is asking, when it is a subagent: its role (else its chat's name). Set by
 * the chat that renders the transcript (`BaseChat` on a subagent's tab), and
 * read by the card. `null` is the chat's own agent.
 */
export const FolderRequesterContext = createContext<string | null>(null);

export function useFolderRequester(): string | null {
  return useContext(FolderRequesterContext);
}

/**
 * The subagent a card came from, when the daemon names it on the preview
 * (`requestedBy`: the role of the subagent whose call is parked, else its chat's
 * name). Only the copy the daemon shows in the chat that delegated the work
 * carries it; the card's home names nobody, and neither does a blank value.
 */
export function requesterOnPreview(preview: ToolPreview | null | undefined): string | null {
  if (preview?.kind !== 'folderAccess') return null;
  const value = preview.requestedBy;
  return typeof value === 'string' && value.trim() ? value.trim() : null;
}

/** ` (requested as /tmp/notes/a.txt)`, the daemon's note on a file it resolved. */
const REQUESTED_AS = /^(.*) \(requested as (.*)\)$/;

/**
 * The folder as the call asked for it. The daemon names the folder it resolved
 * (`/private/tmp/notes` on macOS, where `/tmp` is a link) and writes each file
 * as "resolved (requested as asked)". When a file's two spellings share the part
 * below the folder, the asked-for folder is the asked-for file minus that part.
 * Anything that does not line up keeps the resolved folder.
 */
export function folderAsRequested(folder: string, files: readonly string[]): string {
  const trimmed = folder.replace(/[\\/]+$/, '') || folder;
  for (const file of files) {
    const match = REQUESTED_AS.exec(file);
    if (!match) continue;
    const [, resolved, asked] = match;
    if (!resolved.startsWith(trimmed)) continue;
    const below = resolved.slice(trimmed.length);
    if (below && !/^[\\/]/.test(below)) continue;
    if (!asked.endsWith(below)) continue;
    const askedFolder = asked.slice(0, asked.length - below.length);
    if (askedFolder) return askedFolder;
  }
  return folder;
}

/** The folder as the card's question writes it: as asked, with `~` for home. */
export function folderShown(folder: string, files: readonly string[]): string {
  return tildePath(folderAsRequested(folder, files), homeFolder());
}

/** What the access lets the asker do, as a verb phrase ("read", "read and change"). */
export function accessVerb(access: string): string {
  const normalized = access.trim().toLowerCase();
  if (normalized === 'read-only' || normalized === 'read') return 'read';
  if (normalized === 'read/write' || normalized === 'write' || normalized === 'read-write') {
    return 'read and change';
  }
  return 'use';
}

export const FOLDER_ACCESS_COPY = {
  /** The card's one line. */
  question: ({
    requester,
    access,
    folder,
  }: {
    requester: string | null;
    access: string;
    folder: string;
  }) =>
    requester
      ? `Subagent '${requester}' asks to ${accessVerb(access)} ${folder}`
      : `Allow this chat to ${accessVerb(access)} ${folder}?`,
  /** The Details disclosure that holds the exact paths and the grant's terms. */
  details: 'Details',
  sessionGrant: (access: string, lifetime: string) =>
    `${access.charAt(0).toUpperCase()}${access.slice(1)} access. A session grant includes subfolders. ${lifetime}`,
  onceDescendants:
    'Allow once includes read-only analysis of this folder and its descendants for this operation:',
  onceFiles: 'Allow once applies only to these files in this operation:',
  cancel: 'Cancel operation',
  allowSession: 'Allow for this session',
} as const;
