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

interface ModelsSectionProps {
  setView: (view: View) => void;
}

export default function ModelsSection({ setView }: ModelsSectionProps) {
  const [provider, setProvider] = useState<string | null>(null);
  const [displayModelName, setDisplayModelName] = useState<string>('');
  const [isLoading, setIsLoading] = useState<boolean>(true);
  const { read, getProviders } = useConfig();
  const {
    getCurrentModelDisplayName,
    getCurrentProviderDisplayName,
    currentModel,
    currentProvider,
  } = useModelAndProvider();

  const loadModelData = useCallback(async () => {
    try {
      setIsLoading(true);

      const modelDisplayName = await getCurrentModelDisplayName();
      setDisplayModelName(modelDisplayName);

      const providerDisplayName = await getCurrentProviderDisplayName();
      if (providerDisplayName) {
        setProvider(providerDisplayName);
      } else {
        const providerName = (await read('BIOROUTER_PROVIDER', false)) as string;
        const providers = await getProviders(false);
        const providerDetailsList = providers.filter((provider) => provider.name === providerName);

        if (providerDetailsList.length != 1) {
          toastError({
            title: UNKNOWN_PROVIDER_TITLE,
            msg: UNKNOWN_PROVIDER_MSG,
          });
          setProvider(providerName);
        } else {
          const fallbackProviderDisplayName = providerDetailsList[0].metadata.display_name;
          setProvider(fallbackProviderDisplayName);
        }
      }
    } catch (error) {
      console.error('Error loading model data:', error);
    } finally {
      setIsLoading(false);
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
      loadModelData();
    }
  }, [currentModel, currentProvider, loadModelData]);

  const modelValue = [displayModelName, provider].filter(Boolean).join(' · ');

  return (
    <section id="models" className="pb-8">
      <SettingSection
        id={SETTINGS_SECTION_IDS.defaultModel}
        title={defaultModelCopy.section}
        help={defaultModelCopy.sectionHelp}
      >
        <ModelSettingsButtons
          setView={setView}
          modelValue={isLoading ? <Skeleton className="h-4 w-40" /> : modelValue}
        />
      </SettingSection>

      <LocalModelInventory />

      <SettingSection id={SETTINGS_SECTION_IDS.modelsDangerZone} title={modelsDangerCopy.section}>
        <ResetProviderSection setView={setView} />
      </SettingSection>
    </section>
  );
}
