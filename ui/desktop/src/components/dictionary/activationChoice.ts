import { useCallback, useEffect, useState } from 'react';
import { activateDictionary, readAllConfig, type DictionarySettings } from '../../api';
import { toastError } from '../../toasts';
import { userActionHeaders } from '../../utils/userAction';
import type { DictationStatus } from '../dictation/dictationApi';
import { dictationMessage, SETUP_SUGGEST_LAST_SENTENCE } from '../dictation/copy';
import { loadDictionarySettings, refreshDictionary } from './useDictionary';

const ACTIVATION_KEY = 'dictation_dictionary_activation_version';
type ActivationState = 'loading' | 'first' | 'completed' | 'unavailable';

/** The first verified model install is the person's activation choice (DD-56). */
export function useDictionaryActivationChoice(status: DictationStatus | null) {
  const [checked, setChecked] = useState(true);
  const [pendingModel, setPendingModel] = useState<string | null>(null);
  const [settings, setSettings] = useState<DictionarySettings | null>(null);
  const [activationState, setActivationState] = useState<ActivationState>('loading');
  const [activationFailed, setActivationFailed] = useState(false);
  const [pendingChoice, setPendingChoice] = useState<boolean | null>(null);
  const [savedChoiceDiffered, setSavedChoiceDiffered] = useState(false);
  const [saving, setSaving] = useState(false);
  const showChoice =
    activationState === 'first' && settings?.authority?.raise_settings.allowed === true;
  const needsSave = showChoice && status?.models.some((model) => model.installed) === true;

  useEffect(() => {
    let alive = true;
    void Promise.all([loadDictionarySettings(), readAllConfig()])
      .then(([loaded, config]) => {
        if (!alive) return;
        if (!loaded || config.error || !config.data?.config) {
          setActivationState('unavailable');
          return;
        }
        const marker = config.data.config[ACTIVATION_KEY];
        setSettings(loaded);
        if (marker === undefined || marker === null) {
          setChecked(true);
          setActivationState('first');
        } else if (marker === 1) {
          setChecked(loaded.suggest_from_chats);
          setActivationState('completed');
        } else {
          setActivationState('unavailable');
        }
      })
      .catch(() => {
        if (alive) setActivationState('unavailable');
      });
    return () => {
      alive = false;
    };
  }, []);

  const submitActivation = useCallback(async () => {
    if (!showChoice || saving || !status?.models.some((model) => model.installed)) return;
    const requested = pendingChoice ?? checked;
    setPendingChoice(requested);
    setSaving(true);
    try {
      const result = await activateDictionary({
        body: { suggest_from_chats: requested, first_install: true },
        headers: await userActionHeaders(),
      });
      if (result.error) {
        setActivationFailed(true);
        toastError({
          title: 'Dictionary',
          msg: String(
            (result.error as { message?: string }).message ?? 'The setting could not be saved.'
          ),
        });
        return;
      }
      if (typeof result.data?.suggest_from_chats !== 'boolean') {
        setActivationFailed(true);
        toastError({ title: 'Dictionary', msg: 'The saved choice could not be confirmed.' });
        return;
      }
      setChecked(result.data.suggest_from_chats);
      setSavedChoiceDiffered(result.data.suggest_from_chats !== requested);
      setPendingChoice(null);
      setActivationFailed(false);
      setActivationState('completed');
      await refreshDictionary();
    } catch {
      setActivationFailed(true);
      toastError({ title: 'Dictionary', msg: 'The setting could not be saved.' });
    } finally {
      setSaving(false);
    }
  }, [checked, pendingChoice, saving, showChoice, status]);

  useEffect(() => {
    if (!pendingModel || !status) return;
    if (!showChoice) {
      setPendingModel(null);
      return;
    }
    const failed =
      status.install_job?.model === pendingModel && status.install_job.phase === 'failed';
    if (failed) {
      setPendingModel(null);
      return;
    }
    if (!status.models.some((model) => model.id === pendingModel && model.installed)) return;
    setPendingModel(null);
    void submitActivation();
  }, [pendingModel, showChoice, status, submitActivation]);

  const startInstall = (model: string, firstInstall: boolean) => {
    if (firstInstall && showChoice) setPendingModel(model);
  };
  const changeChecked = (value: boolean) => {
    if (pendingChoice === null) setChecked(value);
  };
  const mode = settings?.agent_writes ?? 'on_request';
  const base = dictationMessage('MSG-SETUP-SUGGEST');
  const first = base.slice(0, base.lastIndexOf(' A term can go into your dictionary'));
  // Two sentences, the InfoTip's limit (owner rule, 2026-11-13: no walls of text).
  const help = `${first} ${SETUP_SUGGEST_LAST_SENTENCE[mode]}`;
  return {
    checked,
    setChecked: changeChecked,
    disabled: !showChoice || saving || pendingChoice !== null,
    showChoice,
    needsSave,
    ready: activationState !== 'loading',
    help,
    startInstall,
    activationFailed,
    savedChoiceDiffered,
    saving,
    retryActivation: submitActivation,
  };
}
