import type { DictionaryAgentWrites, DictionarySettings } from '../../../api';
import { toastError } from '../../../toasts';
import { DICTATION_ACTIONS } from '../../dictation/copy';
import type { DictationStatus } from '../../dictation/dictationApi';
import { DICTIONARY_LABELS, dictionaryMessage, suggestFromChatsHelp } from '../../dictionary/copy';
import { openDictionaryPage } from '../../dictionary/navigation';
import {
  learnFromChatsGate,
  setAgentWrites,
  setLearnFromChats,
  setSuggestFromChats,
  suggestFromChatsGate,
  useDictionarySettings,
  type Outcome,
} from '../../dictionary/useDictionary';
import {
  scanLine,
  startScan,
  stopScan,
  useSuggestionScan,
} from '../../dictionary/useSuggestionScan';
import { Button } from '../../ui/button';
import { SettingRow } from '../../ui/setting-row';
import { Switch } from '../../ui/switch';
import { SettingSelect, type SettingSelectOption } from '../SettingSelect';
import { SETTINGS_SECTION_IDS } from '../settingsSections';

/**
 * The dictionary's rows in **Settings > Models > Speech models** (owner, 2026-11-19: the
 * Dictionary page keeps the words, and Settings keeps how they are found). One group at the
 * end of the section, which the deep link `dictionary` scrolls to:
 *
 * - **Dictionary**, with **Edit your dictionary** (DD-67, PR-20);
 * - **Agent's terms**, the `agent_writes` setting (O-3, DD-53);
 * - **Suggest dictionary words from my chats**, the `dictionary` capability (PR-23, DD-56),
 *   left out while the first install's own choice of it shows above, so one setting never has
 *   two controls on screen;
 * - **Suggest words from my messages**, the chat extractor's `learn_from_chats` (PR-24);
 * - **Recent chats**, with **Find words** (owner, 2026-11-13), while that switch is on and the
 *   caller may write.
 *
 * Every value is the daemon's, read from and written through the Dictionary page's own store,
 * so the page's ⋯ menu, the sidebar and this group show one value. A raise (a higher
 * `agent_writes`, turning a switch on) is refused before the click when the caller has no
 * standing to make it, with the daemon's reason; lowering always works. Absent where managed
 * policy turns the dictionary off, and the options wait until the daemon has answered.
 */
export default function DictionarySettingsRows({
  status,
  activationChoiceShown,
}: {
  status: DictationStatus;
  /** The first install's **Suggest dictionary words from my chats** box shows above. */
  activationChoiceShown: boolean;
}) {
  const allowed = status.policy.dictionary.allowed;
  const settings = useDictionarySettings(allowed);
  if (!allowed) return null;
  return (
    <div
      id={SETTINGS_SECTION_IDS.dictionary}
      className="flex flex-col"
      data-testid="settings-dictionary"
    >
      <SettingRow
        label={DICTIONARY_LABELS.title}
        help={dictionaryMessage('MSG-DICTIONARY-HELP')}
        data-testid="speech-dictionary"
      >
        <Button
          type="button"
          size="sm"
          variant="secondary"
          // Named by what it says, not by the row's label, so the visible words are its name.
          aria-label={DICTIONARY_LABELS.editYourDictionary}
          onClick={openDictionaryPage}
          data-testid="speech-edit-dictionary"
        >
          {DICTIONARY_LABELS.editYourDictionary}
        </Button>
      </SettingRow>
      {settings ? (
        <>
          <AgentTermsRow settings={settings} />
          {activationChoiceShown ? null : <SuggestFromChatsRow settings={settings} />}
          <LearnFromChatsRow settings={settings} />
          {settings.learn_from_chats && settings.authority?.write.allowed === true ? (
            <RecentChatsRow />
          ) : null}
        </>
      ) : null}
    </div>
  );
}

/** A refused write says why, under the setting's own name. */
function reportRefusal(title: string) {
  return (result: Outcome<DictionarySettings>) => {
    if (!result.ok) toastError({ title, msg: result.error.message });
  };
}

const AGENT_WRITES: readonly DictionaryAgentWrites[] = ['suggest', 'on_request', 'add'];
const rank = (mode: DictionaryAgentWrites) => AGENT_WRITES.indexOf(mode);

function AgentTermsRow({ settings }: { settings: DictionarySettings }) {
  const labels = DICTIONARY_LABELS.agentSettings;
  const authority = settings.authority ?? null;
  const canRaise = authority?.raise_settings.allowed === true;
  const current = settings.agent_writes;
  const options = (
    [
      ['suggest', labels.suggest, labels.suggestHint],
      ['on_request', labels.onRequest, labels.onRequestHint],
      ['add', labels.add, labels.addHint],
    ] as const
  ).map(
    ([value, label, description]): SettingSelectOption<DictionaryAgentWrites> => ({
      value,
      label,
      description,
      // A raise the caller may not make shows, but cannot be chosen; a lower mode always can.
      disabled: !canRaise && rank(value) > rank(current),
      testId: `dictionary-agent-writes-${value}`,
    })
  );
  return (
    <SettingRow
      label={labels.title}
      help={dictionaryMessage('MSG-AGENT-WRITES-HELP')}
      disabled={!authority}
      data-testid="dictionary-agent-writes-row"
    >
      <SettingSelect
        options={options}
        value={current}
        disabled={!authority}
        contentClassName="w-80"
        onValueChange={(mode) => {
          if (!authority || mode === current) return;
          if (rank(mode) > rank(current) && !canRaise) return;
          void setAgentWrites(mode).then(reportRefusal(labels.title));
        }}
        data-testid="dictionary-agent-writes"
      />
    </SettingRow>
  );
}

/** **Suggest dictionary words from my chats**: the `dictionary` capability for new chats. */
function SuggestFromChatsRow({ settings }: { settings: DictionarySettings }) {
  const gate = suggestFromChatsGate(settings);
  const canRaise = settings.authority?.raise_settings.allowed === true;
  return (
    <SettingRow
      label={DICTATION_ACTIONS.suggestFromChats}
      help={suggestFromChatsHelp(settings.agent_writes)}
      status={gate.reason ?? undefined}
      data-testid="dictionary-suggest-from-chats"
    >
      <Switch
        checked={settings.suggest_from_chats}
        disabled={gate.disabled}
        onCheckedChange={(checked) => {
          if (!settings.authority || (checked && !canRaise)) return;
          void setSuggestFromChats(checked).then(reportRefusal(DICTIONARY_LABELS.title));
        }}
      />
    </SettingRow>
  );
}

/** **Suggest words from my messages**: the chat extractor's switch, `learn_from_chats`. */
function LearnFromChatsRow({ settings }: { settings: DictionarySettings }) {
  const gate = learnFromChatsGate(settings);
  const canRaise = settings.authority?.raise_settings.allowed === true;
  return (
    <SettingRow
      label={DICTIONARY_LABELS.agentSettings.learnFromChats}
      help={dictionaryMessage('MSG-LEARN-FROM-CHATS-HELP')}
      status={gate.reason ?? undefined}
      data-testid="speech-learn-from-chats"
    >
      <Switch
        checked={settings.learn_from_chats}
        disabled={gate.disabled}
        onCheckedChange={(checked) => {
          if (!settings.authority || (checked && !canRaise)) return;
          void setLearnFromChats(checked).then(reportRefusal(DICTIONARY_LABELS.title));
        }}
      />
    </SettingRow>
  );
}

/**
 * **Recent chats**: **Find words** runs the chat extractor once over the person's recent chats
 * on public models; a running scan shows its progress and **Stop**, then how it ended, or the
 * daemon's refusal. The line is one polite live region that stays mounted, so each change is
 * read out. The button reads **Find words** and is named **Find words in my recent chats**.
 */
function RecentChatsRow() {
  const state = useSuggestionScan(true);
  const running = state.scan?.state === 'running';
  const line = scanLine(state);
  const labels = DICTIONARY_LABELS.scan;
  return (
    <SettingRow
      label={labels.row}
      help={dictionaryMessage('MSG-SCAN-HELP')}
      status={
        <span aria-live="polite" className="text-text-muted" data-testid="dictionary-scan-status">
          {line}
        </span>
      }
      data-testid="dictionary-scan"
    >
      {running ? (
        <Button
          type="button"
          size="sm"
          variant="ghost"
          aria-label={labels.stop}
          disabled={state.stopping}
          onClick={() => void stopScan()}
          data-testid="dictionary-scan-stop"
        >
          {labels.stop}
        </Button>
      ) : (
        <Button
          type="button"
          size="sm"
          variant="secondary"
          aria-label={labels.find}
          disabled={state.starting}
          aria-busy={state.starting || undefined}
          onClick={() => void startScan()}
          data-testid="dictionary-scan-start"
        >
          {labels.findShort}
        </Button>
      )}
    </SettingRow>
  );
}
