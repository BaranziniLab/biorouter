/**
 * Every sentence the renderer shows for dictation, one source.
 *
 * The texts are the message catalog in `docs/dictation/surfaces.md` ("Message
 * catalog"), word for word; `copy.test.ts` reads that table and fails when a
 * row here differs from it. A pull request that needs a new sentence adds its
 * row to the catalog first, then here.
 *
 * Placeholders keep the catalog's spelling, `<host>`, `<n>` and so on, and are
 * filled with {@link fillMessage}. A placeholder the caller does not fill stays
 * as written, which is what the catalog asks for the ones a person types
 * themselves (`--from <file>` in MSG-OFFLINE).
 *
 * Text in backticks is a command or a file name the person types. A component
 * that renders one of these sentences shows those spans as code; the backticks
 * are part of the catalog text, not decoration.
 *
 * Only the sentences the renderer can show live here. The terminal's own
 * sentences are in `crates/biorouter-cli/src/dictation/messages.rs`, and the
 * Dictionary page's, its History tab and the dictation history settings
 * included, are in `components/dictionary/copy.ts`.
 */
export const DICTATION_MESSAGES = {
  'MSG-POLICY': "Dictation is turned off on this computer by your organization's managed policy.",
  'MSG-POLICY-CAPTURE':
    "Your organization's managed policy turns off live microphone dictation on this computer. Transcribing a recording still works.",
  'MSG-POLICY-DOWNLOADS':
    "Your organization's managed policy turns off model downloads on this computer. Speech models come only from <folder>; ask your administrator to place one there.",
  'MSG-POLICY-MODELS':
    "Your organization's managed policy allows only these speech models here: <models>.",
  'MSG-NO-ENGINE': 'This build of Biorouter has no local dictation engine.',
  'MSG-CPU':
    "This processor does not support the instructions local dictation needs (<missing>). Your system's own dictation still works, without your Biorouter dictionary.",
  'MSG-CPU-DESKTOP':
    'To use your Biorouter dictionary, connect this app to a Biorouter server whose processor supports dictation, in Settings > App > Use external server.',
  'MSG-CPU-SERVER':
    "This server's processor cannot run local dictation (it lacks <missing>). Your system's own dictation still works, without your Biorouter dictionary. If you have the Biorouter app on a computer whose processor supports dictation, Dictate to clipboard works there.",
  'MSG-CPU-ROSETTA':
    'This is the Intel build of Biorouter running under Rosetta, which cannot run local dictation on this version of macOS. Install the Apple Silicon build of Biorouter.',
  'MSG-INSTALL-LOCKED':
    'Dictation is not set up on this server. Ask the person who runs it to run `biorouter dictation install`.',
  'MSG-INSTALL-AGENT':
    "Speech models are installed by a person from Biorouter's Settings; an agent cannot install one.",
  'MSG-OFFLINE':
    "This computer cannot reach Biorouter's model repository. On a computer that can, run `biorouter dictation urls` and download the files it lists, copy them here, then run `biorouter dictation install --from <file>` (or put them in <folder> and choose Rescan).",
  'MSG-HASH': "The downloaded file did not match Biorouter's pin, so nothing was installed.",
  // The installer's own sentence (`InstallError::Cancelled`), shown as the daemon sends it; held
  // here so a surface can tell a cancel the person made from a failure (`isCancelledInstall`).
  'MSG-INSTALL-CANCELLED': 'The install was canceled.',
  'MSG-DISK': 'Installing <model> needs <x> GB free on <volume>; <y> GB is available.',
  'MSG-TURBO-CPU':
    'This model takes about <t> s to transcribe <a> s of speech on this computer. Install it anyway?',
  'MSG-TOO-SLOW-SETTINGS':
    '<model> may be too slow for dictation on this computer. After you install it, Test speed measures it. Install it anyway?',
  'MSG-BELOW-MINIMUM-SETTINGS':
    '<model> needs more memory than this computer has, so it may load slowly or not at all. Install it anyway?',
  'MSG-REMOVE-CONFIRM': 'Remove <model> from <folder>?',
  'MSG-REMOVE-CONFIRM-ACTIVE':
    'Remove <model> from <folder>? It is the model in use, so dictation will switch to <next>.',
  'MSG-REMOVE-CONFIRM-LAST':
    'Remove <model> from <folder>? It is the model in use and no other installed model can take its place, so dictation will not work until you install one.',
  'MSG-RESCAN-ADDED': 'Installed <models> from the model folder.',
  'MSG-RESCAN-NONE': 'No new model file was found in <folder>.',
  'MSG-RESCAN-UNKNOWN': 'Not Biorouter model files, so not installed: <names>.',
  'MSG-BENCH':
    '<model> on <backend>, <n> threads: <a> s of speech in <t> s (real time factor <r>, <suitability>).',
  'MSG-SETTINGS-REFUSED': 'The Biorouter service refused this (<code>, HTTP <status>).',
  'MSG-SETTINGS-UNREACHABLE': 'The Biorouter service did not answer. Try again.',
  'MSG-SETTINGS-HELP':
    "Models that turn your speech into text for dictation. Each one downloads once, from the Baranzini Lab's model repository on GitHub.",
  'MSG-FOLDER-HELP':
    "Where this account's speech models are kept. On a computer without internet access, copy the model files here and choose Rescan.",
  'MSG-LANGUAGE-HELP':
    'The language you dictate in. Detect automatically lets a multilingual model decide each time.',
  'MSG-MIC-HELP':
    'The microphone this window records from. Test microphone shows its level for 5 seconds; nothing is recorded or sent.',
  'MSG-SPEED-HELP':
    "Transcribes a 5.5 s sample with the model in use. A model that needs more than half the sample's length is marked slow.",
  'MSG-LANGUAGE-EN':
    '<model> understands only English. Choose a multilingual model for other languages, or set the language to English.',
  'MSG-LANGUAGE-EN-ONLY':
    '<model> understands only English. To dictate in another language, use a multilingual model.',
  'MSG-SERVER-MEMORY':
    'This server does not have enough free memory to load <model> (<x> GB needed, <y> GB free). Try again later, or ask the administrator to install the Light model.',
  'MSG-ENGINE-FAILED':
    'Transcription failed on <where>, so no text was produced. Your recording is kept for 5 minutes.',
  'MSG-LOST': 'Dictation lost its connection to Biorouter, so nothing was inserted. Try again.',
  'MSG-CAP-WARN': 'Dictation stops in 30 seconds. What you said so far will be inserted.',
  'MSG-CAP-STOPPED': 'Dictation stopped at the 10 minute limit and inserted what you said.',
  'MSG-REMOTE-OLD':
    "The Biorouter service on <host> is a version without local dictation. Update it to <this app's version> or later, or turn off Use external server in Settings > App.",
  'MSG-MIC-DENIED-MAC':
    'Microphone access is off for Biorouter. Turn it on in System Settings > Privacy & Security > Microphone, then restart Biorouter.',
  'MSG-MIC-DENIED-WIN':
    'Microphone access is off for Biorouter. Turn on “Let desktop apps access your microphone” in Settings > Privacy & security > Microphone.',
  'MSG-MIC-RESTRICTED': 'Your organization manages microphone access on this computer.',
  'MSG-MIC-GRANTED-FAILED':
    'Biorouter could not open the microphone although macOS allows it. Restart Biorouter and try again.',
  'MSG-MIC-BROWSER':
    "Allow the microphone for this site in your browser's site settings, then try again.",
  'MSG-NO-DEVICE':
    'No microphone found. Connect one or make it the system default, then try again.',
  'MSG-MIC-BUSY': 'The microphone is in use by another app, or the system blocked it.',
  'MSG-INSECURE':
    'Dictation needs a secure address. Your browser blocks the microphone on http://<host>. Open Biorouter through an SSH tunnel (then it is localhost) or through an HTTPS address.',
  'MSG-SHARED-ORIGIN':
    "Your browser's microphone permission covers every page at <origin>, including pages that code or an agent wrote there, such as Biorouter applications and notebook outputs. Choose Allow this time unless you trust every page there.",
  'MSG-413':
    'The proxy in front of Biorouter refused the audio upload (413). Its request size limit is below 64 KB.',
  'MSG-WAIT': 'Waiting for a free connection to Biorouter',
  'MSG-SERVER-BUSY':
    'The server is busy transcribing for other people. Trying again in <n> seconds.',
  'MSG-SERVER-BUSY-FAILED':
    'The server is still busy transcribing for other people, so nothing was inserted. Your recording is kept for 5 minutes.',
  'MSG-RECORDING-FAILED':
    'Biorouter could not read the recording, so nothing was inserted. Try again.',
  'MSG-NOTHING': 'Nothing was heard, so nothing was inserted.',
  'MSG-SILENT': 'The microphone sent only silence. Check that it is not muted.',
  'MSG-NO-AUDIO-YET': 'No audio detected. Check that your microphone is not muted.',
  'MSG-TOO-SHORT': 'That recording was under half a second, so nothing was transcribed.',
  'MSG-APPS':
    "Applications cannot use the microphone themselves. Click into the application's text field, then use Dictate into this application, or use Dictate to clipboard in the Biorouter app (or `biorouter dictate --copy`) and paste. Your system's own dictation also works, without your Biorouter dictionary.",
  'MSG-GNOME':
    "GNOME on Wayland does not let Biorouter set a keyboard shortcut that works in other apps. Use Dictate to clipboard in Biorouter's tray menu (GNOME shows it with the AppIndicator extension), or the microphone button in Biorouter's own composer.",
  'MSG-WINDOW-OPEN':
    'On Windows and Linux, Dictate to clipboard works while a Biorouter window is open. It can be minimized.',
  'MSG-SHORTCUT-TAKEN': 'Another app is already using <shortcut>. Choose a different shortcut.',
  'MSG-NO-TRAY':
    "Biorouter's menu bar icon is off, so Dictate to clipboard is not in its menu. Choose a shortcut below, or turn on Show in menu bar in Settings > App.",
  'MSG-CLIPBOARD-EXPLAIN':
    'Dictate to clipboard puts the text on your system clipboard. After that it is outside Biorouter: any program on this computer can read it, including an agent with shell access in any chat; clipboard managers may keep it; Universal Clipboard or Windows cloud clipboard may sync it to your other devices; and the app you paste into decides where it goes. When Biorouter types into the app you are using, the text is on the clipboard for a moment before Biorouter puts back what was there. The text is also kept in your dictation history, which you can turn off. Your private dictionary terms are used only for a recording you start with Include my private terms.',
  // `<key>` is the key that dictates now: fn, or the chosen modifier plus fn (DD-101, `fnKeyLabel`).
  'MSG-FN-HELP':
    'Hold <key> to talk and let go to insert. Double-tap <key> to dictate hands-free, and double-tap again to stop.',
  // Protocol version 3 also says which one of Shift, Control or Option is held with fn (DD-101).
  'MSG-FN-PERMISSION':
    'Allow Biorouter in Privacy & Security > Input Monitoring so fn can start dictation. Biorouter keeps only whether fn is held and which of Shift, Control or Option is held with it, and drops every other key.',
  // Also the composer's one-time note at the first fn press (DD-98, `biorouter:fn-conflict-shown`):
  // macOS runs its own fn action on every tap, so a double-tap can run it twice.
  'MSG-FN-CONFLICT':
    'macOS also uses fn to <action>, so a hold or a double-tap can do that too. To keep fn for dictation, set Press 🌐 key to Do Nothing in Keyboard settings.',
  'MSG-FN-CONFLICT-UNKNOWN':
    'macOS may also act when you press fn, for example by showing the emoji picker, and a double-tap can do it twice. To keep fn for dictation, set Press 🌐 key to Do Nothing in Keyboard settings.',
  'MSG-FN-FAILED':
    'The fn key listener stopped and could not start again. Turn the switch off and on to retry.',
  'MSG-FN-MISSING': 'This build of Biorouter has no fn key listener, so fn cannot start dictation.',
  'MSG-FN-OTHER-APPS':
    'Hold or double-tap <key> in any app while Biorouter is open. The text goes into that app, or to the clipboard when Biorouter cannot type there.',
  // The alternative key (DD-101, 2026-11-27): a modifier plus fn does what fn does, for when
  // another app also uses fn. `<alt>` is the chosen combination (Control+fn by default) and
  // `<app>` the name from Biorouter's own list of apps known to use fn (`fnCollision.ts`), never
  // text another app reports. The first five are Settings' **Dictation key** and **Key held with
  // fn** rows; the last two are the composer's one-time key notice, with **More in Settings**.
  'MSG-FN-KEY-HELP':
    'Automatic uses fn, or <alt> while another app that uses fn is open, and <alt> always works too. Choose fn or <alt> to use one key only.',
  'MSG-FN-KEY-AUTO': 'Using fn. <alt> works too.',
  'MSG-FN-KEY-SWITCHED': 'Using <alt>, because <app> also uses fn.',
  'MSG-FN-KEY-SHARED': '<app> also uses fn, so one press may start both.',
  'MSG-FN-MODIFIER-HELP': 'Command is left out because macOS uses it for shortcuts.',
  'MSG-FN-USE-ALT-COLLISION':
    '<app> also uses fn. Hold <alt> to dictate in Biorouter, or double-tap it for hands-free.',
  'MSG-FN-USE-ALT-CHOSEN':
    'Biorouter dictates with <alt>. Hold it to talk, or double-tap it for hands-free.',
  // Typing the text where the person is (DD-97, macOS; "Inserting text" and S18 in the catalog's
  // document). MSG-INSERT-LOST is the composer's notice when the field a transcript was for has
  // gone; the next four are main's notifications after a refused insert, each beside the copy
  // that replaced it; the last two are the Settings row **Type into the app you are using**.
  'MSG-INSERT-LOST': 'Biorouter could not find the box this text was for.',
  'MSG-INSERT-NO-PERMISSION':
    'Copied, because Biorouter is not allowed to type into other apps. Allow it in Settings > Models > Speech models.',
  'MSG-INSERT-SECURE':
    'A password field had focus, so Biorouter did not type there. The text is on the clipboard.',
  'MSG-INSERT-TARGET-CHANGED':
    'You switched apps before the text was ready, so Biorouter copied it. Paste with <key>.',
  'MSG-INSERT-FAILED': 'Biorouter could not type the text, so it copied it. Paste with <key>.',
  'MSG-INSERT-HELP':
    'Biorouter pastes the text into the app you are using, then puts back what was on your clipboard. A copied password is cleared, and password fields are skipped.',
  'MSG-INSERT-PERMISSION':
    'Allow Biorouter in Privacy & Security > Accessibility so it can type into other apps.',
  // After a paste that worked, what the helper could not put back on the clipboard (DD-102,
  // `restored` in its `inserted` answer). Nothing shows for `all` or `moved`.
  'MSG-INSERT-CLIPBOARD-PARTIAL':
    'Typed into the app, but Biorouter could not put back everything that was on your clipboard.',
  'MSG-INSERT-CLIPBOARD-KEPT':
    'Typed into the app. The text is still on your clipboard, because Biorouter could not put back what was there.',
  'MSG-INSERT-PASSWORD-CLEARED':
    'Typed into the app. Your clipboard held a password, so Biorouter cleared it instead of putting it back.',
  'MSG-CHORD-HELP':
    'In a message box, tap <chord> to start or stop, or hold it to talk. Windows and Linux do not let apps see the fn key, so Biorouter uses this key combination here.',
  'MSG-SHORTCUT-HELP':
    'A key combination that starts and stops Dictate to clipboard from any app. Leave it empty for none.',
  'MSG-CLIPBOARD-COPIED': 'Copied <n> words. Paste with <key>.',
  'MSG-CLIPBOARD-NOTHING': 'Nothing was heard, so the clipboard was not changed.',
  'MSG-CLIPBOARD-ECHO':
    'Only dictionary words came back, which usually means the recording was silent. Nothing was copied.',
  'MSG-CLIPBOARD-COPY-FAILED':
    'The clipboard did not keep the text. Select it above and press <key>.',
  'MSG-REMOTE-HTTP':
    'Dictation sends your voice to <host> without encryption, as your typed messages already are. Use an https address for this server to protect it. Continue?',
  'MSG-REMOTE-NO-KEY':
    "Dictation on <host> uses only your public dictionary terms until you enter this server's User Action Key in Settings > App > Use external server. Installing models there also needs the key.",
  'MSG-DISCLOSE-LOCAL': 'Speech is transcribed by Biorouter on this computer. Audio is not saved.',
  'MSG-DISCLOSE-REMOTE':
    "Audio is transcribed on <host>, the computer running Biorouter's service. Models install there, not on this device.",
  // One line at rest, the rest in an InfoTip beside it (owner rule "no walls of text";
  // Phase C ruling F-13): a mandated disclosure keeps its core visible.
  'MSG-DISCLOSE-HOST':
    'Your voice is sent to the Biorouter server on <host> and turned into text there.',
  // DD-96: the voice is still never saved; the text now is, in the person's dictation history.
  'MSG-DISCLOSE-HOST-DETAIL':
    'Biorouter does not save your voice or send it anywhere else. The text is kept in your private dictation history on that server, which you can turn off.',
  'MSG-SETUP':
    "Dictation turns your speech into text <where>. It needs a <size> speech model, <name>, downloaded once from the Baranzini Lab's model repository on GitHub into <folder>.",
  'MSG-SETUP-SUGGEST':
    'The agent can propose terms it sees in your chats. A term can go into your dictionary when you ask the agent to add it, or reply to its offer naming that term, even if you decline; everything else waits for you to accept it.',
  'MSG-PRIVATE-DRAFT':
    'This message was dictated with your private dictionary terms turned on, so it may contain them, and <model> is a public model. Send it anyway?',
  'MSG-PRIVATE-DRAFT-SWITCH':
    'This draft was dictated with your private dictionary terms turned on, so it may contain them. If you switch this chat to <model>, a public model, the draft goes there with your next message. Switch anyway?',
  'MSG-PRIVATE-TERMS':
    "Your private dictionary terms may appear in this text, and Biorouter cannot keep them away from public models or from other institutions' models once you paste or post it. Include them?",
  'MSG-CREW-TIP':
    'Dictate. The text goes into the box for you to check; nothing is posted until you press Send.',
  'MSG-RESTART-BUSY': 'Dictation in progress. Restart anyway?',
} as const;

export type DictationMessageId = keyof typeof DICTATION_MESSAGES;

/**
 * The two other forms the catalog gives MSG-SETUP-SUGGEST's last sentence, one
 * per `agent_writes` setting. The base text above is the `on_request` variant,
 * the default the owner chose (O-3). It is the InfoTip of **Suggest dictionary words
 * from my chats**, so it stays two sentences; the chat extractor, which needs no agent,
 * has its own switch and InfoTip (**Suggest words from my messages**, 2026-11-13).
 */
export const SETUP_SUGGEST_LAST_SENTENCE = {
  on_request:
    'A term can go into your dictionary when you ask the agent to add it, or reply to its offer naming that term, even if you decline; everything else waits for you to accept it.',
  suggest: 'Nothing is added until you accept it.',
  add: "The agent's terms are added at once, and you can undo each one; everything else waits for you to accept it.",
} as const;

/**
 * MSG-ENGINE-FAILED's `<where>`, which follows "failed on": "this computer" or
 * "the Biorouter server at <host>". Not MSG-SETUP's, which has no "on" before
 * it; use {@link SETUP_WHERE_THIS_COMPUTER} and {@link setupWhereOnServer}.
 * `dictationStore.ts` `dictationDaemonPlace` says which one applies.
 */
export const WHERE_THIS_COMPUTER = 'this computer';
export const whereOnServer = (host: string): string =>
  host ? `the Biorouter server at ${host}` : 'the Biorouter server';

/** MSG-SETUP's `<where>`, which follows "into text": the forms with "on". */
export const SETUP_WHERE_THIS_COMPUTER = `on ${WHERE_THIS_COMPUTER}`;
export const setupWhereOnServer = (host: string): string => `on ${whereOnServer(host)}`;

/** Button and link labels the catalog and the surfaces document name in bold. */
export const DICTATION_ACTIONS = {
  openSettings: 'Open settings',
  retry: 'Retry',
  saveChoice: 'Save choice',
  installAnyway: 'Install anyway',
  cancel: 'Cancel',
  install: 'Install',
  notNow: 'Not now',
  moreInSettings: 'More in Settings',
  editDictionary: 'Edit your dictionary',
  sendAnyway: 'Send anyway',
  switchAnyway: 'Switch anyway',
  include: 'Include',
  publicTermsOnly: 'Public terms only',
  includePrivateTerms: 'Include my private terms',
  /** The live preview's toolbar button for a Biorouter application's focused field (S15). */
  dictateIntoThisApp: 'Dictate into this application',
  /** The Settings switch for the tray item and the global shortcut (S18). */
  dictateIntoOtherApps: 'Dictate into other apps',
  /** The macOS switch for the fn gestures (hold to talk, double-tap for hands-free). */
  dictateWithFn: 'Dictate with the fn key',
  /** Under **Dictate into other apps** on macOS: the fn gestures there too. */
  useFnInOtherApps: 'Use fn in other apps',
  /**
   * Under **Dictate with the fn key** (DD-101): which press dictates, fn, a modifier plus fn, or
   * Automatic; and the modifier. Their options are {@link FN_KEY_LABELS} and
   * {@link FN_MODIFIER_LABELS}.
   */
  dictationKey: 'Dictation key',
  keyHeldWithFn: 'Key held with fn',
  /**
   * The tray item for 5 minutes after Biorouter typed into another app (DD-102): the person's
   * click writes the last text to the clipboard, nothing else does.
   */
  copyLastDictation: 'Copy last dictation',
  openInputMonitoring: 'Open Input Monitoring',
  openKeyboardSettings: 'Open Keyboard settings',
  /**
   * Under **Dictate into other apps** on macOS (DD-97): fn in other apps and the global shortcut
   * type the text into the app that was in front when the dictation started.
   */
  typeIntoFrontApp: 'Type into the app you are using',
  /** MSG-INSERT-PERMISSION's button: the one path that may ask macOS for Accessibility. */
  openAccessibility: 'Open Accessibility',
  /** MSG-INSERT-LOST's two buttons: put the kept text into this composer, or copy it. */
  insertHere: 'Insert here',
  copy: 'Copy',
  /** The Windows and Linux row that names the composer's key combination. */
  keyboardShortcut: 'Keyboard shortcut',
  setUp: 'Set up dictation',
  showButton: 'Show the microphone button',
  reduceNoise: 'Reduce background noise',
  suggestFromChats: 'Suggest dictionary words from my chats',
  restartAnyway: 'Restart anyway',
  dismiss: 'Dismiss',
} as const;

/**
 * Names and tooltips of the microphone in each phase ("Phases" in
 * `docs/dictation/surfaces.md`), the submit tooltips while dictation holds the
 * composer, the empty field's placeholder, and what the hidden status region
 * announces.
 */
export const DICTATION_LABELS = {
  setUp: 'Set up dictation',
  /** MSG-DISCLOSE-HOST-DETAIL's InfoTip: "About your voice". */
  voicePrivacy: 'your voice',
  activationSaveFailed: 'Dictionary choice could not be saved. Retry to finish setting it up.',
  activationChoicePending: 'Finish setting up Dictionary for new chats.',
  activationSavedChoiceDiffered:
    'Dictionary kept the choice already saved here. Review it in Dictionary settings.',
  installing: 'Downloading speech model… <n>%',
  idle: 'Dictate',
  arming: 'Starting microphone…',
  recording: 'Stop dictation, <time>',
  recordingTooltip: 'Stop and insert · Esc cancels',
  /**
   * After **Dictate** in the idle tooltip while the fn shortcut works (macOS), naming the key
   * that dictates now (DD-101): `<key>` is fn or the chosen modifier plus fn (`fnKeyLabel`).
   */
  fnGestureWith: 'hold or double-tap <key>',
  transcribing: 'Transcribing…',
  finishFirst: 'Finish dictating first',
  listening: 'Listening…',
  announceRecording: 'Recording',
  announceTranscribing: 'Transcribing',
  announceInserted: 'Inserted',
  /** The titles of the two MSG-PRIVATE-DRAFT questions; the catalog gives their sentences. */
  privateDraftTitle: 'Send to a public model?',
  privateDraftSwitchTitle: 'Switch to a public model?',
  /** The muted line that previews what has been heard so far (DD-31). */
  partialLabel: 'Heard so far',
} as const;

/**
 * Fill the `<name>` placeholders of a catalog sentence. A placeholder with no
 * value stays as written.
 */
export function fillMessage(
  template: string,
  values: Readonly<Record<string, string>> = {}
): string {
  return template.replace(/<([^<>]+)>/g, (whole, name: string) =>
    Object.prototype.hasOwnProperty.call(values, name) ? values[name] : whole
  );
}

/** A catalog sentence by id, filled. */
export function dictationMessage(
  id: DictationMessageId,
  values: Readonly<Record<string, string>> = {}
): string {
  return fillMessage(DICTATION_MESSAGES[id], values);
}

/** `m:ss`, the recording timer's form ("Stop dictation, 0:07"). */
export function formatDictationClock(seconds: number): string {
  const whole = Math.max(0, Math.floor(seconds));
  const minutes = Math.floor(whole / 60);
  const rest = whole % 60;
  return `${minutes}:${rest.toString().padStart(2, '0')}`;
}

/**
 * Why the insertion helper did not type the text (`crates/biorouter-textinsert`, its `refused`
 * answer), as `TextInsertService.insert` reports it.
 */
export type InsertRefusalReason = 'no-permission' | 'secure-input' | 'target-changed' | 'failed';

/**
 * The notification main shows beside the fallback copy, per refusal (DD-97). MSG-INSERT-TARGET-CHANGED
 * and MSG-INSERT-FAILED take `<key>`, the paste shortcut (⌘V), as MSG-CLIPBOARD-COPIED does.
 */
export const INSERT_REFUSAL_MESSAGES = {
  'no-permission': 'MSG-INSERT-NO-PERMISSION',
  'secure-input': 'MSG-INSERT-SECURE',
  'target-changed': 'MSG-INSERT-TARGET-CHANGED',
  failed: 'MSG-INSERT-FAILED',
} as const satisfies Readonly<Record<InsertRefusalReason, DictationMessageId>>;

/** The sentence for a refused insert, `<key>` filled with the paste shortcut. */
export function insertRefusalMessage(reason: InsertRefusalReason, pasteKey: string): string {
  return dictationMessage(INSERT_REFUSAL_MESSAGES[reason], { key: pasteKey });
}

/**
 * What the insertion helper did with the clipboard after a paste that worked (DD-102, `restored`
 * in its `inserted` answer): `all` put everything back; `moved` left alone a copy made during the
 * paste; `partial` put back all but what it could not read or what passed its size cap;
 * `kept-text` could not put back what was there, so the dictated text stays; `cleared` cleared a
 * copied password instead of putting it back.
 */
export type InsertRestoreOutcome = 'all' | 'moved' | 'partial' | 'kept-text' | 'cleared';

/** The notification for each restore that did not put everything back. */
export const INSERT_RESTORE_MESSAGES = {
  partial: 'MSG-INSERT-CLIPBOARD-PARTIAL',
  'kept-text': 'MSG-INSERT-CLIPBOARD-KEPT',
  cleared: 'MSG-INSERT-PASSWORD-CLEARED',
} as const satisfies Readonly<
  Record<Exclude<InsertRestoreOutcome, 'all' | 'moved'>, DictationMessageId>
>;

/**
 * The sentence for a restore outcome, or null when there is nothing to say: everything was put
 * back, a copy the person made was left alone, or the helper did not say (an older helper).
 */
export function insertRestoreMessage(restored: InsertRestoreOutcome | undefined): string | null {
  if (restored === 'partial' || restored === 'kept-text' || restored === 'cleared') {
    return dictationMessage(INSERT_RESTORE_MESSAGES[restored]);
  }
  return null;
}

/**
 * The options of **Dictation key** (DD-101): Automatic, fn alone, or the chosen modifier plus fn.
 * `fnKeyLabel` (`utils/dictationFnKey.ts`) names the key that works now with the same words.
 * Command is never one: macOS and every app use it for shortcuts.
 */
export const FN_KEY_LABELS = {
  automatic: 'Automatic',
  fn: 'fn',
  control: 'Control+fn',
  option: 'Option+fn',
  shift: 'Shift+fn',
} as const;

/** The options of **Key held with fn** (DD-101), Control first, the default. */
export const FN_MODIFIER_LABELS = {
  control: 'Control',
  option: 'Option',
  shift: 'Shift',
} as const;

/**
 * MSG-FN-CONFLICT's `<action>`: what macOS does with fn, by `AppleFnUsageType` (System Settings >
 * Keyboard > "Press 🌐 key to"), as the `biorouter-fnkey` helper reads it.
 */
export const FN_USAGE_ACTIONS: Readonly<Record<1 | 2 | 3, string>> = {
  1: 'change the input source',
  2: 'show emoji and symbols',
  3: 'start its own dictation',
};
