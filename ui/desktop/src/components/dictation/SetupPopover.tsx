/**
 * The setup popover (DD-61, DD-92, "The setup popover and disclosure" in
 * `docs/dictation/surfaces.md`), opened by the microphone before a speech model
 * is installed. Its click opens this popover and never starts a download; the
 * download starts only from **Install** here, or in Settings.
 *
 * It names the model, its size and the folder it goes into, says where speech
 * is transcribed as visible text (never an InfoTip, and never "this computer"
 * when the daemon is elsewhere), and offers **Install**, **Not now** and
 * **More in Settings**. While the model installs it shows the progress the
 * microphone's ring shows; completion shows no toast. Once a model is ready,
 * **Edit your dictionary** takes the person to the Dictionary page (PR-20),
 * except under managed `dictionary: false`; the **Suggest dictionary words from
 * my chats** box arrives with PR-23.
 */
import { speechModelsCopy } from '../settings/models/copy';
import {
  installQuestion,
  installSpeechModel,
  useSpeechModels,
} from '../settings/models/speechModelStore';
import { Button } from '../ui/button';
import { DICTIONARY_LABELS } from '../dictionary/copy';
import { openDictionaryPage } from '../dictionary/navigation';
import { Progress } from '../ui/progress';
import {
  DICTATION_ACTIONS,
  DICTATION_LABELS,
  SETUP_WHERE_THIS_COMPUTER,
  dictationMessage,
  fillMessage,
  setupWhereOnServer,
  type DictationMessageId,
} from './copy';
import type { DictationStatus } from './dictationApi';
import { dictationDaemonPlace, hasUsableModel, type DictationDaemonPlace } from './dictationStore';

type SpeechModel = DictationStatus['models'][number];

/** The model the popover offers: the one recommended for this computer, else the one in use. */
export function setupModel(status: DictationStatus | null): SpeechModel | null {
  if (!status) return null;
  return (
    status.models.find((model) => model.recommended && !model.installed) ??
    status.models.find((model) => model.id === status.active_model) ??
    status.models.find((model) => model.recommended) ??
    status.models[0] ??
    null
  );
}

/** MSG-DISCLOSE-LOCAL, MSG-DISCLOSE-REMOTE or MSG-DISCLOSE-HOST, by where the daemon is. */
export function disclosureFor(place: DictationDaemonPlace): string {
  const id: DictationMessageId =
    place.kind === 'this-computer'
      ? 'MSG-DISCLOSE-LOCAL'
      : place.kind === 'external-backend'
        ? 'MSG-DISCLOSE-REMOTE'
        : 'MSG-DISCLOSE-HOST';
  return dictationMessage(id, place.kind === 'this-computer' ? {} : { host: place.host });
}

export interface SetupPopoverProps {
  status: DictationStatus | null;
  /** 0 to 100 while a model installs; null otherwise. */
  progress: number | null;
  onNotNow: () => void;
  onMoreInSettings: () => void;
}

export function SetupPopover({ status, progress, onNotNow, onMoreInSettings }: SetupPopoverProps) {
  const models = useSpeechModels();
  const place = dictationDaemonPlace(status);
  const model = setupModel(status);
  const where =
    place.kind === 'this-computer' ? SETUP_WHERE_THIS_COMPUTER : setupWhereOnServer(place.host);
  const installing = progress !== null;
  // After an install the popover can still be open; the next thing to do is the dictionary,
  // unless managed policy turns it off, where the sidebar row and Settings' link are hidden too.
  const ready =
    !installing && !!status && hasUsableModel(status) && status.policy.dictionary.allowed !== false;
  const pending = model ? !!models.pending[model.id] : false;
  const refusal = model ? models.errors[model.id] : undefined;
  const caution = model && !installing ? installQuestion(model) : null;
  const installingText = fillMessage(DICTATION_LABELS.installing, { n: String(progress ?? 0) });
  const failed =
    status?.install_job?.phase === 'failed' && status.install_job.model === model?.id
      ? status.install_job.error
      : null;

  return (
    <div className="br-dictation-setup" data-testid="dictation-setup">
      <p className="br-dictation-setup__title">{DICTATION_LABELS.setUp}</p>
      {model && status ? (
        <p className="br-dictation-setup__text">
          {dictationMessage('MSG-SETUP', {
            where,
            name: model.display_name,
            size: speechModelsCopy.megabytes(model.bytes),
            folder: status.models_dir,
          })}
        </p>
      ) : (
        <p className="br-dictation-setup__text">{dictationMessage('MSG-SETTINGS-UNREACHABLE')}</p>
      )}
      <p className="br-dictation-setup__privacy" data-testid="dictation-setup-privacy">
        {disclosureFor(place)}
      </p>
      {caution && <p className="br-dictation-setup__warning">{caution.text}</p>}
      {(refusal || failed) && (
        <p className="br-dictation-setup__warning" role="alert">
          {refusal ?? failed}
        </p>
      )}
      {status && !status.install.allowed && status.install.reason ? (
        <p className="br-dictation-setup__warning" role="status">
          {status.install.reason}
        </p>
      ) : null}
      {installing && (
        <div className="br-dictation-setup__progress" data-testid="dictation-setup-progress">
          <span className="text-supporting">{installingText}</span>
          <Progress value={progress} label={installingText} />
        </div>
      )}
      <div className="br-dictation-setup__actions">
        <Button
          type="button"
          variant="link"
          size="xs"
          className="mr-auto h-auto px-0 text-supporting"
          onClick={onMoreInSettings}
        >
          {DICTATION_ACTIONS.moreInSettings}
        </Button>
        {ready ? (
          <Button
            type="button"
            variant="link"
            size="xs"
            className="h-auto px-0 text-supporting"
            data-testid="dictation-setup-edit-dictionary"
            onClick={() => {
              onNotNow();
              openDictionaryPage();
            }}
          >
            {DICTIONARY_LABELS.editYourDictionary}
          </Button>
        ) : null}
        <Button type="button" variant="secondary" size="sm" onClick={onNotNow}>
          {DICTATION_ACTIONS.notNow}
        </Button>
        {!installing && model && status && !model.installed && (
          <Button
            type="button"
            size="sm"
            disabled={pending || !status.install.allowed}
            aria-busy={pending || undefined}
            onClick={() => void installSpeechModel(model.id)}
          >
            {caution ? DICTATION_ACTIONS.installAnyway : DICTATION_ACTIONS.install}
          </Button>
        )}
      </div>
    </div>
  );
}
