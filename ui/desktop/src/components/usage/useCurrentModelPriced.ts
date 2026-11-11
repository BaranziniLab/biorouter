import { useEffect, useState } from 'react';
import { fetchModelPricing } from '../../utils/pricing';
import { useOptionalModelAndProvider } from '../ModelAndProviderContext';

/**
 * Whether the current model has a price, which decides the unit a new limit starts in: dollars
 * when it does (people think of a limit as money), tokens when it does not, since a dollar limit
 * cannot stop a model that has no price. Settings > App > Usage's limit rows read it. `true` until
 * the price is known and outside a model provider, so dollars stay the default.
 */
export function useCurrentModelPriced(): boolean {
  const context = useOptionalModelAndProvider();
  const provider = context?.currentProvider ?? null;
  const model = context?.currentModel ?? null;
  const [priced, setPriced] = useState(true);

  useEffect(() => {
    if (!provider || !model) {
      setPriced(true);
      return;
    }
    let cancelled = false;
    void fetchModelPricing(provider, model).then((pricing) => {
      if (cancelled) return;
      setPriced(
        !!pricing && ((pricing.input_token_cost ?? 0) > 0 || (pricing.output_token_cost ?? 0) > 0)
      );
    });
    return () => {
      cancelled = true;
    };
  }, [provider, model]);

  return priced;
}
