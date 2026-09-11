import { useCallback } from 'react';
import { createBase as apiCreate, deleteBase as apiDelete } from '../../../api';
import { useKnowledge } from '../KnowledgeContext';
import type { KbFormat, Manifest } from '../../../api/types.gen';
import { knowledgeFetch } from './knowledgeRequest';
import { userActionHeaders } from '../../../utils/userAction';

export function useKnowledgeBases() {
  const { refresh, setPrimaryKbId, primaryKbId } = useKnowledge();

  /**
   * Create a base.
   *
   * `format` is passed through only when the caller states one, so a caller
   * that has no opinion still gets the daemon's default rather than the
   * renderer asserting one on its behalf — `CreateBaseBody.format` is
   * `Option<KbFormat>` on the wire for exactly that reason.
   */
  const create = useCallback(
    async (
      id: string,
      name: string,
      options?: { color?: string; format?: KbFormat }
    ): Promise<Manifest | undefined> => {
      const res = await apiCreate({
        throwOnError: true,
        body: {
          id,
          name,
          ...(options?.color ? { color: options.color } : {}),
          ...(options?.format ? { format: options.format } : {}),
        },
      });
      await refresh();
      return res.data;
    },
    [refresh]
  );

  const rename = useCallback(
    async (id: string, name: string, color?: string): Promise<Manifest> => {
      const res = await knowledgeFetch(`/knowledge/bases/${id}`, {
        method: 'PUT',
        headers: {
          'Content-Type': 'application/json',
        },
        body: JSON.stringify({
          name,
          ...(color ? { color } : {}),
        }),
      });

      if (!res.ok) {
        throw new Error(await res.text());
      }

      const manifest = (await res.json()) as Manifest;
      await refresh();
      return manifest;
    },
    [refresh]
  );

  const remove = useCallback(
    async (id: string): Promise<void> => {
      await apiDelete({ throwOnError: true, path: { id }, headers: await userActionHeaders() });
      if (primaryKbId === id) setPrimaryKbId(null);
      await refresh();
    },
    [refresh, primaryKbId, setPrimaryKbId]
  );

  const exportArchive = useCallback(async (id: string, name: string): Promise<void> => {
    const res = await knowledgeFetch(`/knowledge/bases/${id}/export`);
    if (!res.ok) {
      throw new Error(await res.text());
    }

    const blob = await res.blob();
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = `${name || id}.brkb`;
    document.body.appendChild(a);
    a.click();
    a.remove();
    URL.revokeObjectURL(url);
  }, []);

  const importArchive = useCallback(
    async (file: File): Promise<string> => {
      const formData = new FormData();
      formData.append('file', file);

      const res = await knowledgeFetch('/knowledge/bases/import', {
        method: 'POST',
        body: formData,
      });

      if (!res.ok) {
        throw new Error(await res.text());
      }

      const data = (await res.json()) as { id?: string };
      if (!data.id) {
        throw new Error('Imported knowledge base is missing an id');
      }

      await refresh();
      setPrimaryKbId(data.id);
      return data.id;
    },
    [refresh, setPrimaryKbId]
  );

  return { create, rename, remove, exportArchive, importArchive };
}
