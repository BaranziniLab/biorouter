/**
 * Every visible string the .brxt install modal renders, in one place.
 *
 * Tests import these rather than restating them (Crew's `copy.ts` pattern; implementation spec
 * section 1, copy rules). The button labels that name what a click does ("Next: configure",
 * "Next: optional settings", "Install extension") live with that decision in `brxtInstallFlow.ts`.
 *
 * ⚠ **The three tier sentences are privacy disclosures (issue #56 section 13.5)** and are on
 * screen as text, above the Install button, so they read exactly as the design quotes them.
 */
export const BRXT_INSTALL_COPY = {
  // The titles
  addTitle: 'Add extension',
  installTitle: (name: string) => `Install ${name}`,
  configureTitle: (name: string) => `Configure ${name}`,
  fromMarketplace: 'From the Biorouter marketplace',

  // Section 13.5
  tierPrivate:
    'The Biorouter marketplace publishes this name as private, so this extension will be Private: only private models will be able to call it.',
  tierPublicMarketplace:
    'The Biorouter marketplace publishes this name as public. Any model, including commercial models hosted outside your institution, will be able to call this extension.',
  tierPublicFile:
    'Extensions installed from a file are always Public. Any model, including commercial models hosted outside your institution, will be able to call this extension.',

  // The drop zone: one line, and the zone is the control (Knowledge's recipe, as Add skill has it)
  dropZone: 'Drop a .brxt file here',
  dropZoneAnother: 'Drop a different .brxt file here',
  // The button's accessible name: its visible line plus the way in that the drop does not show,
  // so the name always CONTAINS what is on screen (WCAG 2.5.3, label in name).
  dropZoneName: 'Drop a .brxt file here, or choose a file',
  dropZoneAnotherName: 'Drop a different .brxt file here, or choose a file',
  dropBrxt: 'Drop a .brxt file',
  reading: 'Reading bundle…',
  downloading: (name: string) => `Downloading ${name}…`,

  // What the bundle says it is
  detectedFromBundle: 'Detected from bundle',
  skillsIncluded: 'Skills included',
  tools: (count: number) => `${count} tools`,
  skills: (count: number) => `${count} skill${count === 1 ? '' : 's'}`,
  requiredVars: (required: number, optional: number) =>
    `${required} required env var${required === 1 ? '' : 's'}${optional > 0 ? `, ${optional} optional` : ''}`,

  // The configure step
  requiredMark: '*',
  /** Under a required field that is still empty once Install has been pressed. */
  valueRequired: 'Enter a value',
  optionalSettings: 'Optional settings',
  optionalHelp: (name: string, count: number) =>
    `${name} needs no credentials to run. ${count === 1 ? 'This setting is' : `These ${count} settings are`} pre-filled with defaults, so you can install without changing anything.`,
  optionalVariables: (count: number) => `${count} optional variable${count === 1 ? '' : 's'}`,
  prefilled: 'Pre-filled with defaults',

  // Buttons
  cancel: 'Cancel',
  back: 'Back',
  backToMarketplace: 'Back to marketplace',
  retry: 'Retry',
  installing: 'Installing…',
  installExtension: 'Install extension',

  // Failures and results
  noLocalPath:
    'Biorouter is running on another machine, so it cannot read a file you drop here. Copy the file onto that machine and install it with `biorouter extension install <path>`.',
  readFailed: 'Could not read the extension bundle',
  installFailed: 'Could not install the extension bundle',
  registerFailed:
    'Extension installed but failed to register. Try adding it manually from the Extensions tab.',
  installedToast: 'Extension installed and enabled',
} as const;
