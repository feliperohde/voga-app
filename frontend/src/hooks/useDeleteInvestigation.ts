'use client';

import { useCallback } from 'react';
import { toast } from 'sonner';
import { useConfirm } from '@/components/ui/confirm-dialog';
import { useInvestigation } from '@/contexts/InvestigationContext';
import { useLiveState } from '@/hooks/useLiveState';
import { useI18n } from '@/i18n';
import { investigationService, type InvestigationKind } from '@/services/investigationService';

/** What a deletion needs: Home's cards and the open investigation both have it. */
export interface Deletable {
  id: string;
  name: string;
  kind: InvestigationKind;
  /** The documents that go with it, said in the confirmation. */
  documents: number;
}

/**
 * Deletes an investigation or a subject once the person confirms, saying what goes with it, and
 * closes it if it was open. Resolves true when it was deleted.
 */
export function useDeleteInvestigation() {
  const { t } = useI18n();
  const confirm = useConfirm();
  const live = useLiveState();
  const { context, open, refresh } = useInvestigation();

  return useCallback(
    async (investigation: Deletable): Promise<boolean> => {
      const interest = investigation.kind === 'interest';
      // A debate with someone from here is only known to the core, which refuses it the same way
      if (live?.phase === 'recording' && live.investigationId === investigation.id) {
        toast.error(t('investigation.deleteLive'));
        return false;
      }
      const ok = await confirm({
        title: t(interest ? 'interest.deleteTitle' : 'investigation.deleteTitle', { name: investigation.name }),
        description: t(interest ? 'interest.deleteHelp' : 'investigation.deleteHelp', { count: investigation.documents }),
        destructive: true,
        confirmLabel: t('common.delete'),
      });
      if (!ok) return false;
      try {
        await investigationService.remove(investigation.id);
      } catch (error) {
        if (String(error).startsWith('LIVE_IN_USE')) toast.error(t('investigation.deleteLive'));
        else toast.error(t('investigation.deleteFailed'), { description: String(error) });
        return false;
      }
      if (context?.kind === 'investigation' && context.ids.includes(investigation.id)) {
        const ids = context.ids.filter(id => id !== investigation.id);
        open(ids.length ? { kind: 'investigation', ids } : null);
      }
      await refresh().catch(() => {});
      toast.success(t(interest ? 'interest.deleted' : 'investigation.deleted', { name: investigation.name }));
      return true;
    },
    [t, confirm, live, context, open, refresh],
  );
}
