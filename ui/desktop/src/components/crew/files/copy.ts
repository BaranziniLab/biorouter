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

  /** Accessible name of the glyph-only download button; its tooltip names the file. */
  saveAttachment: 'Save attachment',
  saveTooltip: (name: string) => `Save ${name}`,
  previewImage: 'Preview image',
  hidePreview: 'Hide preview',
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

  /** Drag and drop, and pasted files. */
  dropToShare: (channel: string) => `Drop to share in #${channel}`,
  /**
   * While the native Share / Cancel confirmation a drop or paste opened is up (D-DROP). The
   * drop itself shares nothing: the main process's dialog names the file, its size and its full
   * path, and only its Share gives the file capability.
   */
  confirmShare: (name: string) => `To share ${name}, choose Share in the dialog.`,
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
  notSaved: 'Crew can share saved files only. Save it as a file first, then attach it.',

  /** The details pane's Files tab. */
  inProgress: 'In progress',
  uploadedNotSent: 'Uploaded, not sent',
  inThisChannel: 'In this channel',
  attach: 'Attach',
  attachNamed: (name: string) => `Attach ${name}`,
  noFiles: 'No files in this channel yet.',
  attachMismatch: 'This upload no longer matches its shared file, so it wasn’t attached.',
  transfersUnavailable: 'Crew couldn’t check this computer’s transfers.',
} as const;
