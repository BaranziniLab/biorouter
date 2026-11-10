import type { ResourceContents } from '../../api';
import type { DecisionHistoryFilter } from '../../utils/sessionDecisions';

export type ArtifactSource =
  | {
      kind: 'html';
      title: string;
      html: string;
      /** Resource identity for presentation hints, never navigation or read authority. */
      sourceUri?: string;
      preferredWidth?: number;
      preferredHeight?: number;
    }
  | {
      kind: 'externalUrl';
      title: string;
      url: string;
    }
  | {
      kind: 'file';
      title: string;
      path: string;
      /** One-based source location, never part of the file-read IPC path. */
      line?: number;
      /**
       * The path's ONLY evidence is assistant prose — no successful tool call in
       * this transcript wrote it. A model can put any plausible path in
       * backticks, so such a path is a claim, not a receipt, and the panel
       * existence-checks it before it becomes a card (`applyMentionedFileGate`
       * in `BaseChat.tsx`).
       *
       * Absent means CONFIRMED, by one of two routes: a tool call wrote it, or
       * the gate found it on disk. Both clear the flag, so a later read failure
       * on an artifact still carrying it is a path that was never there — which
       * is the difference between "moved, renamed, or deleted" and "the
       * assistant mentioned this path but never created it". Never set to
       * `false`; absence is the confirmed state.
       */
      mentionedOnly?: true;
    }
  | {
      kind: 'mcpResource';
      title: string;
      resource: ResourceContents;
      preferredWidth?: number;
      preferredHeight?: number;
    }
  | {
      /**
       * A chat's decision history (`components/decisions/DecisionHistory.tsx`): the
       * one place individual decision-model calls are listed. One tab per chat
       * (`sessionId` is its key); opening it again with another `filter` re-filters
       * that tab. `openedAt` makes a repeated open of the same filter still apply
       * it. Built by `decisionHistoryArtifact` in `utils/sessionDecisions.ts`.
       */
      kind: 'decisions';
      title: string;
      sessionId: string;
      filter: DecisionHistoryFilter;
      openedAt: number;
    };

export type ArtifactFileEntry = {
  name: string;
  path: string;
  relativePath: string;
  parentPath: string;
  isDirectory: boolean;
  size?: number;
};

export type ArtifactDocumentFormat = 'pdf' | 'docx' | 'xlsx' | 'pptx';

export type ArtifactGitStatus = 'untracked' | 'modified' | 'staged' | 'committed' | 'pushed';

export type ArtifactGitEntry = {
  name: string;
  path: string;
  relativePath: string;
  parentPath: string;
  isDirectory: boolean;
  status: ArtifactGitStatus;
};

export type ArtifactFilePreview =
  | {
      kind: 'text' | 'html';
      title: string;
      path: string;
      mimeType: string;
      text: string;
      size: number;
      revision?: string;
      found: true;
      // For an HTML file, the security-prepared (asset-inlined) HTML the rendered
      // Preview toggle shows. `text` stays the raw source the Raw toggle shows.
      // Only ArtifactViewer sets this (it calls prepareArtifactHtml); the main
      // process read handler does not.
      preparedHtml?: string;
    }
  | {
      kind: 'image';
      title: string;
      path: string;
      mimeType: string;
      size: number;
      revision?: string;
      found: true;
      // **Exactly one of `dataUrl` and `bytes` is set**, decided by size in the
      // main process. A base64 data URL costs ~4/3 of the file as a JS string
      // and pays it twice — once as an IPC structured clone, once as a DOM
      // attribute — and Chromium degrades on multi-megabyte URLs, so anything
      // large arrives as raw bytes for the renderer to wrap in a `blob:` URL.
      // Read them through `imageSourceForPreview`, never directly, so the
      // blob's lifetime is always someone's job.
      dataUrl?: string;
      bytes?: ArrayBuffer;
    }
  | {
      kind: 'document';
      format: ArtifactDocumentFormat;
      title: string;
      path: string;
      mimeType: string;
      data: ArrayBuffer;
      size: number;
      revision?: string;
      extractedText?: string;
      textTruncated?: boolean;
      found: true;
    }
  | {
      kind: 'directory';
      title: string;
      path: string;
      entries: ArtifactFileEntry[];
      found: true;
    }
  | {
      kind: 'gitDirectory';
      title: string;
      path: string;
      branch: string;
      entries: ArtifactGitEntry[];
      found: true;
    }
  | {
      kind: 'binary';
      title: string;
      path: string;
      mimeType: string;
      size: number;
      found: true;
    }
  | {
      kind: 'error';
      title: string;
      path: string;
      /** Friendly, human-readable message — never the raw Node errno string. */
      error: string;
      /** Errno-style code ('ENOENT', 'EACCES', …) when one was recognised. */
      code?: string;
      found: false;
    };

export type PreparedArtifactHtml = {
  html: string;
};
