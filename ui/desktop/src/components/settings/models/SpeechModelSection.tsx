import { useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react';
import { isBrowserSurface } from '../../../utils/surface';
import { AlertTriangle } from '../../icons/app-icons';
import { ManagedPolicyNote } from '../../privacy/ManagedPolicyNote';
import { Button } from '../../ui/button';
import { ConfirmationModal } from '../../ui/ConfirmationModal';
import { Note } from '../../ui/note';
import { Progress } from '../../ui/progress';
import { RowActions, RowContextMenu, type RowActionItem } from '../../ui/row-actions';
import { SettingRow, SettingSection, settingLabelId } from '../../ui/setting-row';
import { Skeleton } from '../../ui/skeleton';
import { Spinner } from '../../ui/spinner';
import { StatusDot } from '../../ui/status-dot';
import { startCapture, type Capture } from '../../dictation/capture';
import { DICTATION_ACTIONS, dictationMessage } from '../../dictation/copy';
import type { DictationStatus } from '../../dictation/dictationApi';
import { isDictationRunning, subscribeDictationRunning } from '../../dictation/dictationSession';
import {
  dictationDaemonPlace,
  hasUsableModel,
  noteDictationInterest,
  readDictationViewer,
  refreshDictationStatus,
  remoteOldValues,
  useDictationStatus,
  writeDictationViewer,
} from '../../dictation/dictationStore';
import { micErrorNotice, type MicErrorNotice } from '../../dictation/micErrors';
import { DICTIONARY_LABELS, dictionaryMessage } from '../../dictionary/copy';
import { openDictionaryPage } from '../../dictionary/navigation';
import { SettingSelect } from '../SettingSelect';
import { SETTINGS_SECTION_IDS } from '../settingsSections';
import { speechModelsCopy as copy } from './copy';
import {
  activeSpeechModel,
  cancelSpeechModelInstall,
  englishOnly,
  installBytes,
  installPercent,
  installQuestion,
  installSpeechModel,
  jobRunning,
  languageOptions,
  removeQuestion,
  removeSpeechModel,
  rescanSpeechModels,
  sectionInstallRefusal,
  setDictationLanguage,
  speechModelRows,
  speedLine,
  testDictationSpeed,
  selectSpeechModel,
  useSpeechModels,
  type SpeechModel,
  type SpeechModelRow,
  type SpeechNotice,
} from './speechModelStore';
import './models.css';
import './speechModels.css';

/**
 * **Settings > Models > Speech models** (DD-66, PR-18): install, use, remove, update and cancel
 * speech models, Rescan the model folder, and set the language and the microphone, on the
 * desktop. One section of the Models tab, between the local model inventory and the danger zone;
 * there is no fourth tab.
 *
 * Hidden on a `serve` browser until PR-21 lands that surface's gates (DD-94: a control appears in
 * the pull request that makes it work end to end); `SpeechModelSection.test.tsx` pins it absent
 * there.
 *
 * The rows follow the Local models pattern (a name over one facts line, the trailing control on the
 * shared right edge, a hover `⋯`), and the privacy line is visible text, never an InfoTip (DD-92).
 * Everything the daemon decides (what is installed, what is in use, whether this caller may
 * install, what managed policy turns off) is read from `GET /dictation/status`; this file never
 * decides it again, and never offers a button the daemon would refuse.
 */
export default function SpeechModelSection() {
  if (isBrowserSurface()) return null;
  return <SpeechModelSectionBody />;
}

const MIC_ROW_ID = 'settings-dictation-microphone';

/** How long Test microphone listens. */
export const MIC_TEST_MS = 5_000;
/** A level above this (about -40 dBFS) counts as sound for the test. */
export const MIC_TEST_HEARD_RMS = 0.01;

/** RMS as a 0 to 100 meter on a -60 to 0 dBFS scale. */
export function meterPercent(rms: number): number {
  if (!(rms > 0)) return 0;
  const db = 20 * Math.log10(rms);
  return Math.max(0, Math.min(100, Math.round(((db + 60) / 60) * 100)));
}

function SpeechModelSectionBody() {
  const { load, status } = useDictationStatus();

  // Mounting the section is a sign of interest (it warms the device list) and a reason to read
  // the status now: what Settings shows should be what the daemon holds when it opens.
  useEffect(() => {
    noteDictationInterest();
    void refreshDictationStatus();
  }, []);

  return (
    <SettingSection
      id={SETTINGS_SECTION_IDS.dictation}
      title={copy.section}
      help={dictationMessage('MSG-SETTINGS-HELP')}
      data-testid="speech-models-section"
    >
      {status ? (
        <SpeechModelRows status={status} />
      ) : load === 'absent' ? (
        <Note tone="warning" role="status" icon={AlertTriangle}>
          {dictationMessage('MSG-REMOTE-OLD', remoteOldValues())}
        </Note>
      ) : load === 'failed' ? (
        <Note
          tone="warning"
          role="alert"
          icon={AlertTriangle}
          action={
            <Button
              type="button"
              size="sm"
              variant="secondary"
              onClick={() => void refreshDictationStatus()}
            >
              {copy.retry}
            </Button>
          }
        >
          {dictationMessage('MSG-SETTINGS-UNREACHABLE')}
        </Note>
      ) : (
        <div className="biorouter-settings-row br-setting-row" data-testid="speech-models-loading">
          <div className="br-setting-row-text gap-1.5">
            <Skeleton className="h-4 w-56" />
            <Skeleton className="h-3 w-40" />
          </div>
        </div>
      )}
    </SettingSection>
  );
}

/** DD-92: where speech is transcribed, in the words for that place. */
function disclosure(status: DictationStatus): string {
  const place = dictationDaemonPlace(status);
  switch (place.kind) {
    case 'this-computer':
      return dictationMessage('MSG-DISCLOSE-LOCAL');
    case 'external-backend':
      return dictationMessage('MSG-DISCLOSE-REMOTE', { host: place.host });
    case 'serve':
      return dictationMessage('MSG-DISCLOSE-HOST', { host: place.host });
  }
}

function SpeechModelRows({ status }: { status: DictationStatus }) {
  const models = useSpeechModels();
  const rows = useMemo(() => speechModelRows(status, models), [status, models]);
  const [confirmInstall, setConfirmInstall] = useState<{
    model: SpeechModel;
    question: SpeechNotice;
  } | null>(null);
  const [confirmRemove, setConfirmRemove] = useState<SpeechModel | null>(null);
  const sectionRefusal = sectionInstallRefusal(status);
  const enabled = status.policy.enabled.allowed;

  const install = useCallback(
    (model: SpeechModel) => {
      const question = installQuestion(model, models.measured);
      if (question) setConfirmInstall({ model, question });
      else void installSpeechModel(model.id);
    },
    [models.measured]
  );

  return (
    <>
      <Note tone="neutral" testId="speech-privacy-line" className="mb-2">
        {disclosure(status)}
      </Note>
      <ManagedPolicyNote policy={status.policy} className="mb-2" />
      {sectionRefusal ? (
        <Note tone="warning" icon={AlertTriangle} testId="speech-install-refusal" className="mb-2">
          {sectionRefusal}
        </Note>
      ) : null}

      {rows.map((row) => (
        <SpeechModelRowView
          key={row.model.id}
          row={row}
          showInstall={!sectionRefusal}
          onInstall={install}
          onRemove={setConfirmRemove}
        />
      ))}

      <FolderRow status={status} enabled={enabled} />
      <LanguageRow status={status} enabled={enabled} />
      <MicrophoneRow status={status} />
      <SpeedRow status={status} />
      <EditDictionaryRow status={status} />

      {/* Not `destructive`: a slow model is a choice, not a deletion. */}
      <ConfirmationModal
        isOpen={confirmInstall !== null}
        title={copy.confirmInstallTitle}
        message={confirmInstall?.question.text ?? ''}
        confirmLabel={DICTATION_ACTIONS.installAnyway}
        cancelLabel={DICTATION_ACTIONS.cancel}
        confirmVariant="default"
        onConfirm={() => {
          const model = confirmInstall?.model;
          setConfirmInstall(null);
          if (model) void installSpeechModel(model.id);
        }}
        onCancel={() => setConfirmInstall(null)}
      />
      <ConfirmationModal
        isOpen={confirmRemove !== null}
        title={copy.confirmRemoveTitle}
        // The model in use is replaced at once by the daemon's next choice, so its question says
        // which one (MSG-REMOVE-CONFIRM-ACTIVE), or that none can take its place.
        message={confirmRemove ? removeQuestion(status, confirmRemove) : ''}
        confirmLabel={copy.remove}
        cancelLabel={DICTATION_ACTIONS.cancel}
        confirmVariant="destructive"
        onConfirm={() => {
          const model = confirmRemove;
          setConfirmRemove(null);
          if (model) void removeSpeechModel(model.id);
        }}
        onCancel={() => setConfirmRemove(null)}
      />
    </>
  );
}

/** The facts line: size, languages, and the notes that apply. */
function factsOf(row: SpeechModelRow): { facts: string; warning: string | null } {
  const { model } = row;
  const parts = [
    copy.megabytes(model.bytes),
    englishOnly(model.id) ? copy.englishOnly : copy.manyLanguages,
  ];
  const warns =
    model.suitability === 'too_slow' ||
    model.suitability === 'below_minimum' ||
    model.suitability === 'unsupported_cpu';
  if (model.recommended && !warns) parts.push(copy.recommended);
  if (model.superseded) parts.push(copy.superseded);
  if (row.elsewhere) parts.push(copy.location[row.elsewhere]);
  // One warning word at most: a model the organization leaves out is that before anything else.
  const warning = row.notAllowed
    ? copy.notAllowed
    : warns
      ? copy.suitability[model.suitability as keyof typeof copy.suitability]
      : null;
  return { facts: parts.join(' · '), warning };
}

function SpeechModelRowView({
  row,
  showInstall,
  onInstall,
  onRemove,
}: {
  row: SpeechModelRow;
  showInstall: boolean;
  onInstall: (model: SpeechModel) => void;
  onRemove: (model: SpeechModel) => void;
}) {
  const { model } = row;
  const use = () => void selectSpeechModel(model.id, model.display_name);
  const items: RowActionItem[] = [];
  if (row.canUse) items.push({ label: copy.use, onSelect: use });
  if (row.canUpdate) items.push({ label: copy.update, onSelect: () => onInstall(model) });
  if (row.canRemove) {
    if (items.length > 0) items.push({ kind: 'separator' });
    items.push({ label: copy.remove, destructive: true, onSelect: () => onRemove(model) });
  } else if (row.elsewhere) {
    items.push({ kind: 'label', label: copy.location[row.elsewhere] });
  }

  const job = row.installing;
  // Before the first byte arrives the daemon reports a downloading job at its total (measured in
  // the dev app: "100%", then "3%" a second later), so a count is shown only below the total.
  const counted = !!job && job.phase === 'downloading' && job.bytes_done < job.bytes_total;
  // The megabytes are the model's own, so the total is the size the facts line showed.
  const bytes = job ? installBytes(job, model.bytes) : null;
  const progressText = !job
    ? null
    : job.phase === 'verifying'
      ? copy.verifying
      : counted && bytes
        ? copy.downloading(
            installPercent(job),
            copy.megabytes(bytes.done).replace(' MB', ''),
            copy.megabytes(bytes.total)
          )
        : copy.downloadingUncounted;
  const busy =
    row.pending === 'install' && !job
      ? copy.starting
      : row.pending === 'cancel'
        ? copy.cancelling
        : row.pending === 'remove'
          ? copy.removing
          : row.pending === 'select'
            ? copy.switching
            : progressText;
  const problem = row.error ?? row.failure;
  const { facts, warning } = factsOf(row);

  let control: React.ReactNode = null;
  if (job) {
    control = (
      <>
        <Progress
          size="thin"
          value={job.phase === 'verifying' ? 100 : installPercent(job)}
          indeterminate={job.phase === 'downloading' && !counted}
          label={progressText ?? copy.downloadingUncounted}
          className="br-speech-model-progress"
          minVisiblePercent={2}
        />
        <Button
          type="button"
          size="sm"
          variant="secondary"
          aria-label={`${copy.cancel} ${model.display_name}`}
          disabled={row.pending === 'cancel'}
          onClick={() => void cancelSpeechModelInstall(model.id)}
        >
          {copy.cancel}
        </Button>
      </>
    );
  } else if (!model.installed && showInstall && row.allowance.allowed) {
    control = (
      <Button
        type="button"
        size="sm"
        variant="secondary"
        // The name follows the visible word, so what a screen reader says contains what it shows.
        aria-label={`${copy.install} ${model.display_name}`}
        disabled={!row.canInstall}
        onClick={() => onInstall(model)}
      >
        {row.pending === 'install' ? <Spinner /> : null}
        {copy.install}
      </Button>
    );
  } else if (row.canUpdate) {
    control = (
      <Button
        type="button"
        size="sm"
        variant="secondary"
        aria-label={`${copy.update} ${model.display_name}`}
        onClick={() => onInstall(model)}
      >
        {copy.update}
      </Button>
    );
  } else if (row.canUse) {
    control = (
      <Button
        type="button"
        size="sm"
        variant="secondary"
        aria-label={`${copy.use} ${model.display_name}`}
        disabled={!!row.pending}
        onClick={use}
      >
        {copy.use}
      </Button>
    );
  }

  const body = (
    <div
      className="biorouter-settings-row br-setting-row"
      data-testid={`speech-model-${model.id}`}
      data-active={row.active ? 'true' : undefined}
    >
      <div className="br-setting-row-text">
        <div className="br-setting-row-head">
          <span className="br-setting-row-label text-label text-text-default">
            {model.display_name}
          </span>
        </div>
        {busy ? (
          <span role="status" className="br-local-model-busy text-supporting text-text-muted">
            <Spinner size={14} />
            <span className="min-w-0 [overflow-wrap:anywhere]">{busy}</span>
          </span>
        ) : problem ? (
          <span role="alert" className="text-supporting text-text-warning [overflow-wrap:anywhere]">
            {problem}
          </span>
        ) : (
          <span className="text-supporting text-text-muted">
            {facts}
            {warning ? (
              <>
                {' · '}
                <span className="text-text-warning">{warning}</span>
              </>
            ) : null}
          </span>
        )}
      </div>
      <div className="br-setting-row-control br-local-model-actions">
        {items.length > 0 ? (
          <RowActions
            menu={items}
            size="sm"
            menuLabel={copy.more(model.display_name)}
            meta={
              model.installed ? (
                <span className="br-local-model-installed">
                  <StatusDot tone={row.active ? 'success' : 'idle'} />
                  {row.active ? copy.inUse : copy.installed}
                </span>
              ) : undefined
            }
          />
        ) : null}
        {control}
      </div>
    </div>
  );

  return items.length > 0 ? <RowContextMenu items={items}>{body}</RowContextMenu> : body;
}

function showInFolderLabel(): string {
  const platform = window.electron?.platform;
  if (platform === 'darwin') return copy.showInFinder;
  if (platform === 'win32') return copy.showInExplorer;
  return copy.showInFolder;
}

function FolderRow({ status, enabled }: { status: DictationStatus; enabled: boolean }) {
  const models = useSpeechModels();
  // Only for the daemon this app started: an external backend's folder is on another computer,
  // and the main process refuses it too.
  const canReveal =
    dictationDaemonPlace(status).kind === 'this-computer' &&
    typeof window.electron?.dictationShowModelsFolder === 'function';
  // The status lists speech models only; a placed voice detector is named as the catalog names it.
  const byId = (id: string) =>
    status.models.find((model) => model.id === id)?.display_name ??
    id.replace(/^silero-vad-(v[\w.]+)$/, `${copy.voiceDetector} $1`);
  const notice = models.rescan.notice;

  return (
    <SettingRow
      label={copy.folder}
      help={dictationMessage('MSG-FOLDER-HELP')}
      data-testid="speech-model-folder"
      status={
        <>
          <span className="br-speech-row-line">
            <span className="br-speech-path">{status.models_dir}</span>
            {' · '}
            <span className="br-speech-account">{copy.account(status.account)}</span>
          </span>
          {notice ? (
            <span
              role="status"
              className={`br-speech-row-line ${
                notice.tone === 'warning' ? 'text-text-warning' : 'text-text-muted'
              }`}
            >
              {notice.text}
            </span>
          ) : null}
        </>
      }
    >
      <div role="group" className="br-speech-row-actions">
        {canReveal ? (
          <Button
            type="button"
            size="sm"
            variant="secondary"
            onClick={() => void window.electron.dictationShowModelsFolder!(status.models_dir)}
          >
            {showInFolderLabel()}
          </Button>
        ) : null}
        <Button
          type="button"
          size="sm"
          variant="secondary"
          disabled={!enabled || models.rescan.running}
          onClick={() => void rescanSpeechModels(status.models_dir, byId)}
        >
          {models.rescan.running ? <Spinner /> : null}
          {copy.rescan}
        </Button>
      </div>
    </SettingRow>
  );
}

function LanguageRow({ status, enabled }: { status: DictationStatus; enabled: boolean }) {
  const models = useSpeechModels();
  const value = models.language.pending ?? status.language;
  const active = activeSpeechModel(status);
  // An English only model in use takes `en` alone (MSG-LANGUAGE-EN), so the menu offers nothing
  // the daemon would refuse after the round trip.
  const englishModel = active && englishOnly(active.id) ? active : null;
  const options = useMemo(
    () => languageOptions(copy.autoLanguage, status.language, !!englishModel),
    [status.language, englishModel]
  );
  // English is chosen and is the one value there is: nothing to open the menu for.
  const nothingElse = !!englishModel && options.length === 1;
  const line = models.language.error ? (
    <span role="alert" className="text-text-warning">
      {models.language.error}
    </span>
  ) : englishModel && status.language !== 'en' ? (
    // The saved language is one the model in use refuses, so every dictation would be refused
    // too (a multilingual model removed or changed, or a hand edited config.yaml).
    <span role="status" className="text-text-warning">
      {dictationMessage('MSG-LANGUAGE-EN', { model: englishModel.display_name })}
    </span>
  ) : englishModel ? (
    dictationMessage('MSG-LANGUAGE-EN-ONLY', { model: englishModel.display_name })
  ) : undefined;
  return (
    <SettingRow
      label={copy.language}
      help={dictationMessage('MSG-LANGUAGE-HELP')}
      data-testid="speech-language"
      disabled={!enabled}
      status={line}
    >
      <SettingSelect
        options={options}
        value={value}
        disabled={!enabled || models.language.pending !== null || nothingElse}
        onValueChange={(next) => {
          if (next !== status.language) void setDictationLanguage(next);
        }}
        triggerClassName="w-44"
      />
    </SettingRow>
  );
}

interface Microphone {
  deviceId: string;
  label: string;
}

/** The audio inputs the browser lists, without Chromium's two aliases of the default. */
async function listMicrophones(): Promise<Microphone[]> {
  if (typeof navigator === 'undefined' || !navigator.mediaDevices?.enumerateDevices) return [];
  try {
    const devices = await navigator.mediaDevices.enumerateDevices();
    return devices
      .filter((device) => device.kind === 'audioinput')
      .filter((device) => device.deviceId !== 'default' && device.deviceId !== 'communications')
      .filter((device) => device.deviceId !== '')
      .map((device, index) => ({
        deviceId: device.deviceId,
        label: device.label || copy.unnamedMicrophone(index + 1),
      }));
  } catch {
    return [];
  }
}

type MicTest =
  | { phase: 'idle' }
  | { phase: 'listening'; startedAt: number; level: number; heard: boolean }
  | { phase: 'heard' }
  | { phase: 'silent' }
  | { phase: 'failed'; notice: MicErrorNotice };

function MicrophoneRow({ status }: { status: DictationStatus }) {
  const { micStatus } = useDictationStatus();
  const [devices, setDevices] = useState<Microphone[]>([]);
  const [choice, setChoice] = useState(() => readDictationViewer().deviceId ?? '');
  const [test, setTest] = useState<MicTest>({ phase: 'idle' });
  const capture = useRef<Capture | null>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const mounted = useRef(true);

  const refreshDevices = useCallback(() => {
    void listMicrophones().then((next) => {
      if (mounted.current) setDevices(next);
    });
  }, []);

  useEffect(() => {
    mounted.current = true;
    refreshDevices();
    const media = typeof navigator !== 'undefined' ? navigator.mediaDevices : undefined;
    media?.addEventListener?.('devicechange', refreshDevices);
    return () => {
      mounted.current = false;
      media?.removeEventListener?.('devicechange', refreshDevices);
      if (timer.current) clearTimeout(timer.current);
      // Leaving the page ends a test: the tracks stop and the system's microphone light goes off.
      capture.current?.cancel();
      capture.current = null;
    };
  }, [refreshDevices]);

  const captureAllowed = status.policy.enabled.allowed && status.policy.capture.allowed;
  const viewer = readDictationViewer();
  const options = useMemo(() => {
    const list = [
      { value: '', label: copy.systemDefault },
      ...devices.map((device) => ({
        value: device.deviceId,
        label: device.label,
      })),
    ];
    // A device chosen earlier that is not plugged in now keeps its name in the menu.
    if (choice && !list.some((option) => option.value === choice)) {
      list.push({
        value: choice,
        label: viewer.deviceLabel ?? copy.unnamedMicrophone(list.length),
      });
    }
    return list;
  }, [devices, choice, viewer.deviceLabel]);

  const choose = (deviceId: string) => {
    setChoice(deviceId);
    const device = devices.find((candidate) => candidate.deviceId === deviceId);
    writeDictationViewer(
      deviceId
        ? { deviceId, deviceLabel: device?.label }
        : { deviceId: undefined, deviceLabel: undefined }
    );
  };

  const finish = (next: MicTest) => {
    if (timer.current) clearTimeout(timer.current);
    timer.current = null;
    capture.current?.cancel();
    capture.current = null;
    if (mounted.current) {
      setTest(next);
      // The first grant gives the devices their names.
      refreshDevices();
    }
  };

  const runTest = async () => {
    if (test.phase === 'listening') return;
    const desktop = !isBrowserSurface();
    // The first use on macOS: the prompt comes from the person's click and names Biorouter.
    if (
      desktop &&
      window.electron?.platform === 'darwin' &&
      micStatus === 'not-determined' &&
      typeof window.electron?.dictationMicRequest === 'function'
    ) {
      const granted = await window.electron.dictationMicRequest().catch(() => false);
      if (!granted) {
        finish({
          phase: 'failed',
          notice: micErrorNotice(
            { name: 'NotAllowedError' },
            { platform: 'darwin', surface: 'desktop', osStatus: 'denied' }
          ),
        });
        return;
      }
    }
    const startedAt = performance.now();
    let heard = false;
    setTest({ phase: 'listening', startedAt, level: 0, heard: false });
    const settings = readDictationViewer();
    try {
      // Only the level is read; the samples are dropped as they arrive. Nothing is recorded,
      // kept or sent (DD-66).
      const started = await startCapture({
        deviceId: settings.deviceId,
        deviceLabel: settings.deviceLabel,
        noiseSuppression: settings.noiseReduction,
        onChunk: (_pcm, level) => {
          if (level.rms > MIC_TEST_HEARD_RMS) heard = true;
          if (mounted.current) {
            setTest((current) =>
              current.phase === 'listening'
                ? { ...current, level: meterPercent(level.rms), heard }
                : current
            );
          }
        },
      });
      if (!mounted.current) {
        started.cancel();
        return;
      }
      capture.current = started;
      void started.ended.then((why) => {
        if (why === 'track-ended' && capture.current === started) {
          finish({
            phase: 'failed',
            notice: micErrorNotice(
              { name: 'NotFoundError' },
              {
                platform: window.electron?.platform ?? 'unknown',
                surface: 'desktop',
                osStatus: 'unknown',
              }
            ),
          });
        }
      });
      const left = Math.max(0, MIC_TEST_MS - (performance.now() - startedAt));
      timer.current = setTimeout(() => finish({ phase: heard ? 'heard' : 'silent' }), left);
    } catch (error) {
      const osStatus = desktop
        ? await Promise.resolve(window.electron?.dictationMicStatus?.()).catch(() => undefined)
        : undefined;
      finish({
        phase: 'failed',
        notice: micErrorNotice(error, {
          platform: window.electron?.platform ?? 'unknown',
          surface: desktop ? 'desktop' : 'browser',
          osStatus: osStatus ?? 'unknown',
          host: typeof location !== 'undefined' ? location.host : undefined,
        }),
      });
    }
  };

  const seconds =
    test.phase === 'listening'
      ? Math.max(1, Math.ceil((MIC_TEST_MS - (performance.now() - test.startedAt)) / 1000))
      : 0;

  const line = !captureAllowed ? (
    copy.managed
  ) : test.phase === 'listening' ? (
    <>
      <Progress
        size="thin"
        value={test.level}
        label={copy.level}
        className="br-speech-meter"
        data-testid="speech-mic-meter"
      />
      <span className="br-speech-row-line">{copy.listening(seconds)}</span>
    </>
  ) : test.phase === 'heard' ? (
    <span role="status">{copy.heard}</span>
  ) : test.phase === 'silent' ? (
    <span role="status" className="text-text-warning">
      {dictationMessage('MSG-NO-AUDIO-YET')}
    </span>
  ) : test.phase === 'failed' ? (
    <span role="alert" className="text-text-warning">
      {test.notice.text}
      {test.notice.action === 'open-settings' &&
      typeof window.electron?.dictationOpenMicSettings === 'function' ? (
        <>
          {' '}
          <Button
            type="button"
            variant="link"
            className="h-auto p-0 text-supporting"
            onClick={() => void window.electron.dictationOpenMicSettings!()}
          >
            {DICTATION_ACTIONS.openSettings}
          </Button>
        </>
      ) : null}
    </span>
  ) : undefined;

  return (
    <SettingRow
      label={copy.microphone}
      help={dictationMessage('MSG-MIC-HELP')}
      data-testid="speech-microphone"
      controlId={MIC_ROW_ID}
      disabled={!captureAllowed}
      status={line}
    >
      <div role="group" className="br-speech-row-actions">
        <SettingSelect
          options={options}
          value={choice}
          // The row's label and the chosen device, as every settings select is named.
          aria-labelledby={settingLabelId(MIC_ROW_ID)}
          disabled={!captureAllowed || test.phase === 'listening'}
          onValueChange={choose}
          triggerClassName="w-44"
        />
        <Button
          type="button"
          size="sm"
          variant="secondary"
          disabled={!captureAllowed || test.phase === 'listening'}
          onClick={() => void runTest()}
        >
          {copy.testMicrophone}
        </Button>
      </div>
    </SettingRow>
  );
}

function SpeedRow({ status }: { status: DictationStatus }) {
  const models = useSpeechModels();
  const { bench } = models;
  // A bench decodes on the worker a dictation in this window uses, so it waits for the dictation.
  const dictating = useSyncExternalStore(subscribeDictationRunning, isDictationRunning);
  const ready =
    status.policy.enabled.allowed &&
    status.engine.compiled &&
    status.engine.state !== 'unsupported_cpu' &&
    hasUsableModel(status) &&
    !jobRunning(status.install_job) &&
    !dictating;
  // Only the model in use: a time measured for one no longer in use, or no longer installed, is
  // not this row's, and a run in progress replaces the line until it answers.
  const shown = speedLine(status, models);
  const line = !shown ? undefined : 'error' in shown ? (
    <span role="alert" className="text-text-warning">
      {shown.error}
    </span>
  ) : (
    <span role="status">
      {dictationMessage('MSG-BENCH', {
        model: shown.model.display_name,
        backend: copy.backend[status.engine.backend ?? ''] ?? status.engine.backend ?? '',
        n: String(status.engine.threads),
        a: (shown.result.elapsed_ms / Math.max(shown.result.rtf, 1e-6) / 1000).toFixed(1),
        t: (shown.result.elapsed_ms / 1000).toFixed(2),
        r: shown.result.rtf.toFixed(2),
        suitability: copy.suitabilityWord[shown.model.suitability] ?? '',
      })}
    </span>
  );
  return (
    <SettingRow
      label={copy.speed}
      help={dictationMessage('MSG-SPEED-HELP')}
      data-testid="speech-speed"
      disabled={!status.policy.enabled.allowed}
      status={line}
    >
      <Button
        type="button"
        size="sm"
        variant="secondary"
        // Named by what it says, not by the row's label, so the visible words are in the name.
        aria-label={bench.running ? copy.testing : copy.testSpeed}
        disabled={!ready || bench.running}
        onClick={() => void testDictationSpeed(activeSpeechModel(status)?.id ?? null)}
      >
        {bench.running ? <Spinner /> : null}
        {bench.running ? copy.testing : copy.testSpeed}
      </Button>
    </SettingRow>
  );
}

/**
 * **Edit your dictionary** (DD-67, PR-20): the link to the Dictionary page, which then opens the
 * sidebar's Components group by the current route rule. Absent where managed policy turns the
 * dictionary off, as the sidebar's row is.
 */
function EditDictionaryRow({ status }: { status: DictationStatus }) {
  if (!status.policy.dictionary.allowed) return null;
  return (
    <SettingRow
      label={DICTIONARY_LABELS.title}
      help={dictionaryMessage('MSG-DICTIONARY-HELP')}
      data-testid="speech-dictionary"
    >
      <Button
        type="button"
        size="sm"
        variant="secondary"
        onClick={openDictionaryPage}
        data-testid="speech-edit-dictionary"
      >
        {DICTIONARY_LABELS.editYourDictionary}
      </Button>
    </SettingRow>
  );
}
