/**
 * The artifact panel's user-facing strings (spec 3.8, copy rules in spec 1).
 *
 * Sentence case, "chat" not "conversation", no dashes as punctuation, the
 * typographic ellipsis. Tests import these rather than restating them, so a
 * wording change is one edit.
 */
export const PREVIEW_COPY = {
  // The tab strip and its actions.
  tabList: 'Open artifact previews',
  overflow: 'Show all previews',
  closeTab: (title: string) => `Close ${title}`,
  openExternally: 'Open externally',
  closePanel: 'Close preview panel',
  moreActions: 'More actions',
  quoteSelection: 'Quote selection',
  // The accessible name the CI preview job clicks (`scripts/preview-panel-e2e.mjs`).
  annotate: 'Send a region to the chat',
  cancelAnnotate: 'Cancel region selection',
  revealInFinder: 'Reveal in Finder',
  showInFolder: 'Show in folder',
  foldPreview: 'Fold the preview to its tabs',
  unfoldPreview: 'Show the preview',
  resizePanel: 'Resize artifact panel',
  // The refresh control that replaced the "Update ready" text button. Its
  // accessible name stays the state word; the sentence is its tooltip.
  updateReady: 'Update ready',
  updateReadyHint: 'A newer version is ready. Refresh after you finish here.',
  retryUpdate: 'Retry update',
  retryUpdateHint: 'Could not read the update. The previous version is still shown.',

  // The preview bar.
  copy: 'Copy',
  copied: 'Copied',
  copyFailed:
    'Biorouter could not write to the clipboard. Select the text and copy it with your keyboard.',
  copyFailedShort: 'Copy failed',
  viewLabel: 'View',
  preview: 'Preview',
  table: 'Table',
  raw: 'Raw',
  fit: 'Fit',
  actualSize: '1:1',
  rowsShown: (shown: number, total: number) =>
    `${shown.toLocaleString()} of ${total.toLocaleString()} rows`,
  rowsCapHint: (hidden: number) =>
    `${hidden.toLocaleString()} more row${hidden === 1 ? '' : 's'} are not shown. Switch to Raw for the full file.`,
  lineOutsideFile: 'The requested source line is outside this file.',
  lineTooLarge: 'Source-line highlighting is unavailable for this large file.',

  // Documents.
  pages: (count: number) => `${count.toLocaleString()} page${count === 1 ? '' : 's'}`,
  sheet: (index: number) => `Sheet ${index}`,
  sheetPicker: 'Sheet',
  documentFormat: {
    pdf: 'PDF',
    docx: 'Word document',
    xlsx: 'Workbook',
    pptx: 'Presentation',
  },

  // Bodies.
  fileNotAvailable: 'File not available',
  imageUndecodable: 'This image could not be decoded. It may be truncated or corrupt.',
  binaryFallbackReason:
    'This file can’t be previewed here. Open it in the app your system uses for this file type.',
  openFile: 'Open the file',
  openHere: 'Open here',
  openInBrowser: 'Open in default browser',
  openInBrowserShort: 'Open in browser',
  pageUnavailable: 'This page can’t be opened here. Open it in your browser instead.',
  selectFile: 'Select a file',
  noMatchingFiles: 'No matching files',
  emptyFolder: 'This folder is empty',
  filterFiles: 'Filter files…',
  filterRepository: 'Filter repository files',
  filterFolder: 'Filter folder files',
  currentlyViewing: 'Currently viewing',
  kernel: 'Kernel',
  kernelHelp: (kernel: string) => `Written for the ${kernel} kernel.`,
  notebookUnreadable: (reason: string) => `Could not preview this notebook: ${reason}`,
} as const;
