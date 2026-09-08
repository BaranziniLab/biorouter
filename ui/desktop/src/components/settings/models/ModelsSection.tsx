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

  return (
    <section id="models" className="pb-8">
      <div className="biorouter-settings-section">
        <div className="biorouter-settings-section-header">
          <h2 className="text-caps text-text-muted">Current Model</h2>
        </div>
        <div className="biorouter-settings-list">
          <div className="biorouter-settings-row px-3 py-2.5">
            {isLoading ? (
              <>
                <Skeleton className="h-5 w-48" />
                <Skeleton className="mt-1.5 h-4 w-32" />
              </>
            ) : (
              <div className="animate-in fade-in duration-100">
                <p className="text-label text-text-default">{displayModelName}</p>
                <p className="mt-0.5 text-supporting text-text-muted">{provider}</p>
              </div>
            )}
          </div>
        </div>
        {/* The button strip is a SECTION action, not a row's trailing control:
            nested inside the row it inherited the row's hover wash, so pointing
            anywhere near the buttons washed the whole model readout. */}
        <ModelSettingsButtons setView={setView} />
      </div>

      <LocalModelInventory />

      <div className="biorouter-settings-section">
        <div className="biorouter-settings-section-header">
          <h2 className="text-caps text-text-muted">Reset</h2>
        </div>
        <div className="biorouter-settings-list">
          <ResetProviderSection setView={setView} />
        </div>
      </div>
    </section>
  );
}
