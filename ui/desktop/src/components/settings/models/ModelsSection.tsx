import { useEffect, useState, useCallback, useRef } from 'react';
import { View } from '../../../utils/navigationUtils';
import ModelSettingsButtons from './subcomponents/ModelSettingsButtons';
import { useConfig } from '../../ConfigContext';
import {
  UNKNOWN_PROVIDER_MSG,
  UNKNOWN_PROVIDER_TITLE,
  useModelAndProvider,
} from '../../ModelAndProviderContext';
import { toastError } from '../../../toasts';
import ResetProviderSection from '../reset_provider/ResetProviderSection';
import LocalModelInventory from './LocalModelInventory';
import { Skeleton } from '../../ui/skeleton';
import { SettingSection } from '../../ui/setting-row';
import { SETTINGS_SECTION_IDS } from '../settingsSections';
import { defaultModelCopy, modelsDangerCopy } from './copy';
import { joinModelValue, knownDefaultModelValue } from './defaultModelValue';

interface ModelsSectionProps {
  setView: (view: View) => void;
}

export default function ModelsSection({ setView }: ModelsSectionProps) {
  const { read, getProviders, providersList } = useConfig();
  const {
    getCurrentModelDisplayName,
    getCurrentProviderDisplayName,
    currentModel,
    currentProvider,
  } = useModelAndProvider();

  // The Model row's value. `null` is "not known yet" and is the only state that draws a skeleton:
  // the app already holds the model and provider in force, so a visit to this tab starts on the
  // words and the daemon's answer, which says the same, replaces them in place (V4-R7-1).
  const [modelValue, setModelValue] = useState<string | null>(() =>
    knownDefaultModelValue(currentModel, currentProvider, providersList)
  );

  const loadModelData = useCallback(async () => {
    let modelDisplayName = '';
    try {
      modelDisplayName = await getCurrentModelDisplayName();

      let providerLabel: string | null;
      const providerDisplayName = await getCurrentProviderDisplayName();
      if (providerDisplayName) {
        providerLabel = providerDisplayName;
      } else {
        const providerName = (await read('BIOROUTER_PROVIDER', false)) as string;
        const providers = await getProviders(false);
        const providerDetailsList = providers.filter((provider) => provider.name === providerName);

        if (providerDetailsList.length != 1) {
          toastError({
            title: UNKNOWN_PROVIDER_TITLE,
            msg: UNKNOWN_PROVIDER_MSG,
          });
          providerLabel = providerName;
        } else {
          providerLabel = providerDetailsList[0].metadata.display_name;
        }
      }
      // One write once both halves are known, so a row that already shows a value never reads
      // the new model beside the old provider.
      setModelValue(joinModelValue(modelDisplayName, providerLabel));
    } catch (error) {
      console.error('Error loading model data:', error);
      setModelValue((shown) => shown ?? joinModelValue(modelDisplayName, null));
    }
  }, [read, getProviders, getCurrentModelDisplayName, getCurrentProviderDisplayName]);

  useEffect(() => {
    loadModelData();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const prevModelRef = useRef<string | null>(null);
  const prevProviderRef = useRef<string | null>(null);

  useEffect(() => {
    if (
      currentModel &&
      currentProvider &&
      (currentModel !== prevModelRef.current || currentProvider !== prevProviderRef.current)
    ) {
      prevModelRef.current = currentModel;
      prevProviderRef.current = currentProvider;
      // A switch made while this tab is open changes the words now, from what the app holds, and
      // when it cannot say, the row waits on a skeleton: the old model beside a new switch would
      // be wrong, and a wrong word is worse than a short wait.
      setModelValue(knownDefaultModelValue(currentModel, currentProvider, providersList));
      loadModelData();
    }
  }, [currentModel, currentProvider, providersList, loadModelData]);

  return (
    <section id="models" className="pb-8">
      <SettingSection
        id={SETTINGS_SECTION_IDS.defaultModel}
        title={defaultModelCopy.section}
        help={defaultModelCopy.sectionHelp}
      >
        <ModelSettingsButtons
          setView={setView}
          modelValue={modelValue === null ? <Skeleton className="h-4 w-40" /> : modelValue}
        />
      </SettingSection>

      <LocalModelInventory />

      <SettingSection id={SETTINGS_SECTION_IDS.modelsDangerZone} title={modelsDangerCopy.section}>
        <ResetProviderSection setView={setView} />
      </SettingSection>
    </section>
  );
}
