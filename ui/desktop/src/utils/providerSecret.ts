import { revealProviderSecret } from '../api';
import { userActionHeaders } from './userAction';

export async function loadProviderSecret(provider: string, key: string): Promise<string> {
  const response = await revealProviderSecret({
    body: { provider, key },
    headers: await userActionHeaders(),
    throwOnError: true,
  });
  if (typeof response.data !== 'string') throw new Error('Could not read the saved key.');
  return response.data;
}
