'use client';

import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from '@/components/ui/dialog';
import { InvestigationWizard } from '@/components/InvestigationWizard';
import { useI18n } from '@/i18n';
import type { InvestigationKind } from '@/services/investigationService';

/** `kind`: a political investigation or an interest, its first step skipped. */
export function NewInvestigationDialog({ open, kind, onOpenChange }: { open: boolean; kind?: InvestigationKind; onOpenChange: (open: boolean) => void }) {
  const { t } = useI18n();
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[90vh] max-w-2xl overflow-y-auto">
        <DialogHeader>
          <DialogTitle>{t(kind === 'interest' ? 'interest.new' : 'investigation.new')}</DialogTitle>
          <DialogDescription>{t('investigation.newHelp')}</DialogDescription>
        </DialogHeader>
        {/* A new wizard each time it opens */}
        {open && <InvestigationWizard kind={kind} onDone={() => onOpenChange(false)} />}
      </DialogContent>
    </Dialog>
  );
}
