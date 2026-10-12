/**
 * The words of Settings > Models and its dialogs (codex simplicity redesign, spec §3.13).
 *
 * Tests import these instead of retyping them. A control's accessible name is a deliberate
 * choice here, not a side effect: `changeName` and `manageName` keep the names the daemon's
 * docs, the browser-surface tests and the focus-return tests already use, so a visible
 * "Change…" can sit on a button that is still announced as "Switch models".
 *
 * Help strings are InfoTip text: full sentences, at most two, about 160 characters. Labels and
 * one-line descriptions take no trailing period.
 */

export const defaultModelCopy = {
  section: 'Default model',
  sectionHelp:
    'New chats start on this model. A chat can use another one from its composer without changing this.',
  model: 'Model',
  change: 'Change…',
  /** The button's accessible name. Pinned by hostManagedSettings and the focus-return tests. */
  changeName: 'Switch models',
  providers: 'Providers',
  manage: 'Manage…',
  /** The button's accessible name. Pinned by hostManagedSettings. */
  manageName: 'Configure providers',
} as const;

/** `256k`: a context window in thousands of tokens, the unit every row, summary and detail uses. */
const contextAmount = (tokens: number | null | undefined) => {
  if (typeof tokens !== 'number' || !Number.isFinite(tokens) || tokens <= 0) return null;
  return `${Math.round(tokens / 1024).toLocaleString()}k`;
};

/**
 * `128 GB`: a memory amount in the unit the download sizes use ("7.6 GB" in a model's option and
 * its button). The daemon counts memory in GiB, and the first-run panel printed "128 GiB unified
 * memory, 32 GiB recommended" a line under "Download and start (7.6 GB)" (vision FR-R4-5,
 * principle 5). A Mac's own About window calls the same 128 GiB "128 GB", so the word is the one
 * a person already reads. Every memory amount the renderer writes goes through this.
 */
const gigabytes = (gib: number) => `${gib} GB`;

export const localModelsCopy = {
  section: 'Local models',
  sectionHelp:
    'Models that run on this computer, so chats with them never leave it. Each one downloads once.',
  /** "2 of 7 installed", the section's count. */
  installedCount: (installed: number, total: number) => `${installed} of ${total} installed`,
  refresh: 'Refresh local models',
  install: 'Install',
  /** The quiet status an installed row shows at rest; the ⋯ takes its place on hover. */
  installed: 'Installed',
  installing: 'Installing',
  more: (name: string) => `More actions for ${name}`,
  viewInfo: 'View info',
  warmUp: 'Warm up',
  delete: 'Delete',
  retry: 'Retry',
  /** The one warning word a row shows, and only when the model is bigger than the machine's memory. */
  largeForThisMac: 'Large for this Mac',
  largeForThisComputer: 'Large for this computer',
  /** The daemon's `below_minimum`: under the least memory the model loads in. */
  belowMinimum: (gib: number) => `Below the ${gigabytes(gib)} minimum`,
  /** The daemon's `cpu_only`: no GPU acceleration here (an Intel Mac, the Linux CPU build). */
  cpuOnly: 'CPU only, slow',
  /**
   * Whether a model fits this machine, in words (`fitVerdict` / `fitLine` in `localModelMemory`).
   * It replaces the daemon's sentence ("Recommended for this machine: 128 GiB apple_unified meets
   * the 32 GiB recommendation."), which repeated its row's label and printed a machine id on a
   * reading surface (vision FR-R2-6).
   */
  fitsThisMac: 'Fits this Mac',
  fitsThisComputer: 'Fits this computer',
  /** No reading: "VRAM not detected", "Unified memory not detected". */
  memoryNotDetected: (memory: string) =>
    `${memory.charAt(0).toUpperCase()}${memory.slice(1)} not detected`,
  /**
   * What the daemon's memory figure measures on this machine (`accelerator_memory_kind`), as a
   * noun after an amount: "64 GB unified memory", "64 GB system RAM", "24 GB VRAM".
   */
  memoryKind: {
    unified: 'unified memory',
    /** `cpu_only`: an Intel Mac, or the Linux CPU build of llama-server. */
    systemRam: 'system RAM',
    vram: 'VRAM',
  },
  /** A figure the daemon could not read: "unknown VRAM". */
  unknownMemory: (memory: string) => `unknown ${memory}`,
  /** What the figure means on this kind of machine: the last sentence of a memory warning. */
  memoryMeaning: {
    unified: 'On Apple Silicon, unified memory is the relevant GPU memory budget.',
    cpuOnly:
      'Llama Server has no GPU acceleration on this machine (an Intel Mac, or the Linux CPU build), so models run on the processor out of system RAM, at a few tokens per second.',
    vram: 'On Windows, Linux and other discrete-GPU systems, this means VRAM, not regular system RAM.',
  },
  /**
   * The sentence a confirmation says before a model the daemon did not rate suitable installs,
   * starts or warms up (`llamaMemoryWarning`), one per verdict. `detected` is "16 GB unified
   * memory"; `meaning` is one of {@link memoryMeaning}.
   */
  memoryWarning: {
    /** No "recommends N GB": a 64 GB Intel Mac clears every figure and is still a processor. */
    cpuOnly: (name: string, detected: string, meaning: string) =>
      `${name} would run without GPU acceleration on this machine (${detected}). ${meaning} Expect several minutes before the first reply of each chat.`,
    belowMinimum: (
      name: string,
      minimum: number,
      recommended: number,
      detected: string,
      meaning: string
    ) =>
      `${name} needs at least ${gigabytes(minimum)} of GPU-addressable memory (${gigabytes(recommended)} recommended). This machine reports ${detected}, so it may fail to load or run very slowly. ${meaning}`,
    aboveRecommendation: (name: string, recommended: number, detected: string, meaning: string) =>
      `${name} recommends ${gigabytes(recommended)} GPU-addressable memory. This machine reports ${detected}. ${meaning}`,
    unknown: (name: string, minimum: number, recommended: number, meaning: string) =>
      `Biorouter could not detect this machine's GPU memory. ${name} needs at least ${gigabytes(minimum)} of GPU-addressable memory (${gigabytes(recommended)} recommended). ${meaning}`,
  },
  /**
   * On Windows, system RAM does not count toward a model's GPU memory. An InfoTip on first run's
   * Memory row, and a line of the warm-up dialog. Not said on a CPU-only machine, where system
   * RAM is exactly what counts.
   */
  windowsVram:
    'On Windows, make sure free VRAM is high enough for the model and context window; regular system RAM does not satisfy the GPU memory recommendation.',
  /** The Windows note's InfoTip names it "About Windows memory". */
  windowsVramLabel: 'Windows memory',
  /**
   * Ollama's verdict on this machine (`GET /ollama/recommended`), one for both recommended models.
   * The word is the Llama Server rows' (`belowMinimum`, `cpuOnly`, `memoryNotDetected`); these are
   * its InfoTip, in GB, in place of the daemon's GiB paragraph (FR-R4-5, principle 2).
   */
  ollamaFitHelp: {
    belowMinimum: (detected: string) =>
      `This computer reports ${detected}. The recommended models may fail to load or run very slowly, and there is no smaller one to offer.`,
    cpuOnly:
      'Ollama runs models on the processor on an Intel Mac, at a token or two per second. A cloud or institutional provider is much faster here.',
    unknown: (minimum: number) =>
      `The recommended models need at least ${gigabytes(minimum)} of GPU memory. Without a supported GPU, Ollama runs them on the processor at a token or two per second.`,
  },
  /** The Ollama fit word's InfoTip names it "About memory". */
  memoryLabel: 'memory',
  /** `128 GB`, the unit of every memory amount (see `gigabytes`). */
  gigabytes,
  /** `128 GB unified memory`, `24 GB VRAM`. */
  memoryAmount: (gib: number, memory: string) => `${gigabytes(gib)} ${memory}`,
  recommendedMemory: (gib: number) => `${gigabytes(gib)} recommended`,
  /** A row of the info dialog: `32 GB GPU-addressable memory`. */
  gpuMemory: (gib: number) => `${gigabytes(gib)} GPU-addressable memory`,
  /** Row facts joined with a middle dot: `4.2 GB · 128k context`. */
  contextWindow: (tokens: number | null | undefined) => {
    const amount = contextAmount(tokens);
    return amount ? `${amount} context` : null;
  },
  /** A Details row's value: `256k`, beside the label "Context". */
  contextAmount,
  close: 'Close',
  warmUpModel: 'Warm up model',
  installModel: 'Install model',
  /**
   * The states beside `installed`. A model has ONE of these words (`stateWord` in
   * `localModelFacts`), and the row's trailing slot, its facts line and the info dialog's Status
   * row all read it: a model that is partly downloaded in Ollama with its fallback GGUF whole is
   * "Installed" in all three, not "Installed", "Partial download" and "Fallback ready".
   */
  status: {
    /** Installed, and Ollama holds the file: Delete removes it from Ollama. */
    inOllama: 'Downloaded in Ollama',
    partial: 'Partial download',
    needsDownload: 'Needs download',
  },
  /** The info dialog's rows. The dialog is where the machine facts live, not the row. */
  detail: {
    status: 'Status',
    family: 'Family',
    download: 'Download',
    expectedSpeed: 'Expected speed',
    context: 'Context',
    minimumMemory: 'Minimum memory',
    recommendedMemory: 'Recommended memory',
    detectedMemory: 'Detected memory',
    recommendation: 'Recommendation',
    /** First run's Details row: the fit, the memory reported and the memory recommended, in one. */
    memory: 'Memory',
    ollamaModel: 'Ollama model',
    /**
     * The model file Llama Server downloads when it cannot load Ollama's copy. It read "Fallback
     * GGUF, may download" on first run, a label that read as a note (vision FR-R3-6).
     */
    fallbackDownload: 'Fallback download',
    officialUrl: 'Official URL',
    modelStore: 'Model store',
    modelBlob: 'Model blob',
  },
  /** A model bigger than this computer's memory recommendation asks before it installs or loads. */
  confirmResources: {
    title: 'Continue anyway?',
    installAnyway: 'Install anyway',
    warmUpAnyway: 'Warm up anyway',
    fallbackConfirm: 'Continue',
    cancel: 'Cancel',
  },
  /**
   * Both sentences name the model by its display name, never `ollama_name`: that tag may be a
   * different build from the file the row lists, and naming it told a person the wrong model was
   * about to go (#393).
   */
  confirmDelete: {
    title: 'Delete this local model?',
    message: (name: string) =>
      `${name} will be removed from the local model inventory. You can download it again later.`,
    /** A model whose file Ollama holds: Delete removes every tag that uses that file. */
    ollamaMessage: (name: string) =>
      `The ${name} model file will be deleted from Ollama, under every tag that uses it. You can download it again later.`,
    confirm: 'Delete',
    cancel: 'Cancel',
  },
  /**
   * What a delete reports. A model Ollama holds is deleted by the tags that hold its file, which
   * may not be `ollama_name` (#393), so the toast names them and a failure says how to finish it.
   */
  deleteResult: {
    deleted: 'Local model deleted',
    removed: (name: string) => `${name} was removed from local storage.`,
    removedTags: (name: string, tags: readonly string[]) =>
      `${name} was removed from local storage (Ollama ${tags.length === 1 ? 'tag' : 'tags'} ${tags.join(', ')}).`,
    failed: 'Local model delete failed',
    ollamaNotRunning: 'Ollama must be running to delete a model it holds.',
    noTag: (name: string, path?: string | null) =>
      `Biorouter could not tell which Ollama tag holds the ${name} model file${path ? ` (${path})` : ''}, so it deleted nothing. Run \`ollama list\` and remove that tag with \`ollama rm <tag>\`.`,
    tagRefused: (tag: string) => `Ollama could not delete ${tag}`,
  },
  /** An Ollama pull that failed, in Ollama's own words. */
  pullFailed: (tag: string, reason: string) => `Ollama could not pull ${tag}: ${reason}`,
} as const;

export const modelsDangerCopy = {
  section: 'Danger zone',
  row: 'Reset provider and model',
  rowHelp: 'Clears the provider and model new chats start on, so you can choose them again.',
  open: 'Reset…',
  confirmTitle: 'Reset provider and model?',
  confirmMessage:
    'New chats will have no provider or model until you choose them again. If no defaults are available, Biorouter opens the welcome screen.',
  confirm: 'Reset',
  cancel: 'Cancel',
} as const;

export const leadWorkerCopy = {
  /** The dialog's title and the switch's name: pinned by the tests and the composer chip. */
  title: 'Lead/worker mode',
  mode: 'Lead/worker mode',
  modeHelp:
    'A stronger lead model plans and recovers from failures, and a faster worker model does the routine steps.',
  leadModel: 'Lead model',
  leadModelHelp: 'The strong model for initial planning and for recovery after failures.',
  workerModel: 'Worker model',
  workerModelHelp: 'The fast model for routine execution steps.',
  leadPlaceholder: 'Select lead model…',
  workerPlaceholder: 'Select worker model…',
  customPlaceholder: 'Type model name here',
  backToList: 'Back to model list',
  leadTurns: 'Initial lead turns',
  leadTurnsHelp: 'How many turns the lead model takes at the start.',
  failureThreshold: 'Failure threshold',
  failureThresholdHelp: 'How many failures in a row hand the work back to the lead model.',
  fallbackTurns: 'Fallback turns',
  fallbackTurnsHelp: 'How many turns the lead model takes after a fallback.',
  loading: 'Loading…',
  cancel: 'Cancel',
  /** The button's name: pinned by hostManagedSettings and the dialog's tests. */
  save: 'Save settings',
} as const;

export const switchModelCopy = {
  /** The dialog's title: pinned by the Settings focus-return test and the browser-surface tests. */
  title: 'Switch models',
  provider: 'Provider',
  providerPlaceholder: 'Provider, type to search',
  model: 'Model',
  modelPlaceholder: 'Select a model, type to search',
  loadingModels: 'Loading models…',
  customModelName: 'Custom model name',
  customPlaceholder: 'Type model name here',
  backToList: 'Back to model list',
  recommended: 'Recommended',
  quickStart: 'Quick start guide',
  cancel: 'Cancel',
  /** The confirm's name: pinned by every SwitchModelModal test. */
  select: 'Select model',
  switching: 'Switching…',
} as const;
