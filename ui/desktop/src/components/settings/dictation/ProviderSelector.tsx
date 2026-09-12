import { useState, useEffect } from 'react';
import { ChevronDown } from '../../icons/app-icons';
import { DictationProvider, DictationSettings } from '../../../hooks/useDictationSettings';
import { useConfig } from '../../ConfigContext';
import { ElevenLabsKeyInput } from './ElevenLabsKeyInput';
import { ProviderInfo } from './ProviderInfo';
import { VOICE_DICTATION_ELEVENLABS_ENABLED } from '../../../updates';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from '../../ui/dropdown-menu';

interface ProviderSelectorProps {
  settings: DictationSettings;
  onProviderChange: (provider: DictationProvider) => void;
}

export const ProviderSelector = ({ settings, onProviderChange }: ProviderSelectorProps) => {
  const [hasOpenAIKey, setHasOpenAIKey] = useState(false);
  const [showProviderDropdown, setShowProviderDropdown] = useState(false);
  const { getProviders } = useConfig();

  useEffect(() => {
    const checkOpenAIKey = async () => {
      try {
        const providers = await getProviders(false);
        const openAIProvider = providers.find((p) => p.name === 'openai');
        setHasOpenAIKey(openAIProvider?.is_configured || false);
      } catch (error) {
        console.error('Error checking OpenAI configuration:', error);
        setHasOpenAIKey(false);
      }
    };

    checkOpenAIKey();
  }, [getProviders]);

  const handleOpenChange = (open: boolean) => {
    setShowProviderDropdown(open);
    if (open) {
      void (async () => {
        try {
          const providers = await getProviders(true);
          const openAIProvider = providers.find((p) => p.name === 'openai');
          const isConfigured = !!openAIProvider?.is_configured;
          setHasOpenAIKey(isConfigured);
        } catch (error) {
          console.error('Error checking OpenAI configuration:', error);
          setHasOpenAIKey(false);
        }
      })();
    }
  };

  const handleProviderChange = (provider: DictationProvider) => {
    onProviderChange(provider);
    setShowProviderDropdown(false);
  };

  const getProviderLabel = (provider: DictationProvider): string => {
    switch (provider) {
      case 'openai':
        return 'OpenAI Whisper';
      case 'elevenlabs':
        return 'ElevenLabs';
      default:
        return 'None (disabled)';
    }
  };

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between py-2 px-2 hover:bg-background-muted rounded-element transition-all">
        <div>
          <h3 className="text-text-default">Dictation provider</h3>
          <p className="text-xs text-text-muted max-w-md mt-[2px]">
            Choose how voice is converted to text
          </p>
        </div>
        <DropdownMenu open={showProviderDropdown} onOpenChange={handleOpenChange}>
          <DropdownMenuTrigger asChild>
            <button
              type="button"
              className="flex items-center gap-2 px-3 py-1.5 text-sm border border-border-subtle rounded-element hover:border-border-default transition-colors text-text-default bg-background-default"
              aria-label="Choose dictation provider"
            >
              {getProviderLabel(settings.provider)}
              <ChevronDown className="w-4 h-4" aria-hidden="true" />
            </button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" className="w-48">
            <DropdownMenuRadioGroup
              value={settings.provider ?? ''}
              onValueChange={(value) => handleProviderChange(value as DictationProvider)}
            >
              <DropdownMenuRadioItem value="openai">
                <span>OpenAI Whisper</span>
                {!hasOpenAIKey && <span className="text-xs text-text-muted">(not configured)</span>}
              </DropdownMenuRadioItem>
              {VOICE_DICTATION_ELEVENLABS_ENABLED && (
                <DropdownMenuRadioItem value="elevenlabs">ElevenLabs</DropdownMenuRadioItem>
              )}
            </DropdownMenuRadioGroup>
          </DropdownMenuContent>
        </DropdownMenu>
      </div>

      {VOICE_DICTATION_ELEVENLABS_ENABLED && settings.provider === 'elevenlabs' && (
        <ElevenLabsKeyInput />
      )}

      <ProviderInfo provider={settings.provider} />
    </div>
  );
};
