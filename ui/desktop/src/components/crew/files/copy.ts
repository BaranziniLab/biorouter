/**
 * The files area's strings (ui-redesign-spec, copy deck "Composer and files": `file.*`,
 * `transfer.*`, `ref.*`, `upload.*`).
 *
 * Tests import these instead of retyping them. A string marked pinned is asserted by a
 * regression test; change it only together with that test. Transfer state words
 * ("Uploading 42%", "Paused", …) are not here: they come from `state/crewStatus.ts`, which
 * every surface that names a transfer state shares.
 */
export const filesCopy = {
  /** Pinned (CFT and `files/useCrewUpload.test.tsx`): an upload with no verified privacy. */
  privacyPending: 'Refresh the workspace to verify connection privacy before uploading.',
  uploadFailed: 'The upload couldn’t start.',
  tooLarge: (name: string) => `${name} is larger than 1 GB. Crew can share files up to 1 GB.`,

  /**
   * The card's controls are named for the file they act on, so two cards never read the same
   * ("Save attachment" did, for every card, Q3-13). `which` is the post time when another loaded
   * card has the same name ("Save counts.csv, 6:54 PM"): see {@link filesCopy.which}.
   */
  saveNamed: (name: string, which = '') => `Save ${name}${which}`,
  previewNamed: (name: string, which = '') => `Preview ${name}${which}`,
  hidePreviewNamed: (name: string, which = '') => `Hide preview of ${name}${which}`,
  /** The ⋯ menu's first item, so the menu is never one of IDs only (Q3-26). */
  saveItem: (name: string) => `Save ${name}…`,
  /** What tells two same-named cards apart in their controls' names: ", 6:54 PM". */
  which: (posted: string) => (posted ? `, ${posted}` : ''),
  /** The Files tab's "In this channel" row: who shared it and when (Q3-13). */
  sharedBy: 'Shared by',
  /** The submenu that holds Copy file ID and Copy SHA-256 (Q3-26). */
  copyForSupport: 'Copy for support',
  /** A file whose metadata has not loaded (or could not load). */
  attachment: 'Attachment',
  copyFileId: 'Copy file ID',
  copySha: 'Copy SHA-256',
  fileActions: (name: string) => `More actions for ${name}`,
  downloadFailed: 'The download couldn’t start.',
  detailsFailed: 'Crew couldn’t load this file’s details.',
  previewFailed: 'The preview couldn’t load.',
  previewAlt: (name: string) => `Preview of ${name}`,

  pause: 'Pause',
  pauseNamed: (name: string) => `Pause ${name}`,
  /** The upload chip's Pause tooltip: the glyph alone read as "Paused?" (Q3-16). */
  pauseUpload: 'Pause upload',
  /**
   * An upload chip in its first second, or before it has moved 1%: a spinner and this word,
   * never "0%" beside a pause glyph, which read as paused (Q3-16).
   */
  uploading: 'Uploading…',
  resume: 'Resume…',
  resumeNamed: (name: string) => `Resume ${name}`,
  removeFromList: 'Remove from list',
  removeFromListHelp: 'Removes the record on this computer. Shared files and saved downloads stay.',
  transferFailed: 'Crew couldn’t update that transfer.',

  serverPath: 'server path',
  serverPathLoading: 'Server path',
  notUploaded: 'Not uploaded',
  notUploadedHelp:
    'Crew shares the path only. It doesn’t check that the file exists or grant access to it.',

  copied: 'Copied',
  copyFailed: 'Copy failed',

  /**
   * Drag and drop, and pasted files. A drop ATTACHES: it puts the file in the message being
   * written, and nothing reaches the channel until Send (Q3-14). "Drop to share" promised more.
   */
  dropToAttach: (channel: string) => `Drop to attach in #${channel}`,
  /**
   * While the native Share / Cancel confirmation a drop or paste opened is up (D-DROP). The
   * drop itself shares nothing: the main process's dialog names the file, its size and its full
   * path, and only its Share gives the file capability. It names no file: a dropped shortcut
   * resolves to its target in the dialog, so a name here could disagree with the dialog's
   * (Q3-25).
   */
  confirmShare: 'To share it, choose Share in the dialog.',
  /** A drop or paste while that confirmation is already open: nothing new opens. */
  finishConfirming: 'Finish the open share confirmation first.',
  /**
   * The older path, for a desktop build without the drop confirmation: the drop opened the
   * secure file window with nothing selected, so this says what to do there (Q2-16). `folder` is
   * the name of the folder the file is in, when the preload could tell; it is display only and
   * goes nowhere else.
   */
  chooseInWindow: (name: string, folder?: string) =>
    `A file window opened. Select ${name}${folder ? ` (it’s in ${folder})` : ''} there and choose Open to share it.`,
  /** A drop or paste while a file window is already open: nothing new opens. */
  finishChoosing: 'Finish choosing a file in the open file window first.',
  oneAtATime: 'Crew shares one file at a time.',
  folderRefused: 'Crew shares files, not folders. Choose a file inside the folder.',
  /** Pinned twin of the main process's sentence for a forged drop or a paste with no file (Q3-25). */
  notSaved: 'Crew can share saved files only. Save it as a file first, then share it again.',

  /** The details pane's Files tab. */
  inProgress: 'In progress',
  /**
   * The files attached to the message being written: uploaded, and waiting for Send (Q3-03).
   * Its rows offer nothing; the composer chip's × is the way to take one out.
   */
  inYourMessage: 'In your message, not sent yet',
  /**
   * An upload that finished but is in no message and not in the composer: a picked or dropped
   * file whose chip was removed, or one whose view reset before it landed. Never a file that
   * a loaded message already carries.
   */
  uploadedNotSent: 'Uploaded, not sent',
  inThisChannel: 'In this channel',
  attach: 'Attach',
  attachNamed: (name: string) => `Attach ${name}`,
  noFiles: 'No files in this channel yet.',
  attachMismatch: 'This upload no longer matches its shared file, so it wasn’t attached.',
  transfersUnavailable: 'Crew couldn’t check this computer’s transfers.',
} as const;
