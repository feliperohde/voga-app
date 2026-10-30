'use client';

import { PageHeader } from '@/components/PageHeader';
import { ConnectionsMap } from '@/components/ConnectionsMap';
import { useInvestigation } from '@/contexts/InvestigationContext';
import { useI18n } from '@/i18n';

export default function MapPage() {
  const { t } = useI18n();
  const { scope } = useInvestigation();
  const ids = scope.map(i => i.id);
  return (
    <div className="flex h-screen flex-col">
      <PageHeader title={t('nav.map')} />
      {ids.length > 1 && <p className="border-b border-gray-100 px-4 py-2 text-xs text-gray-500 sm:px-8">{t('map.sharedHelp')}</p>}
      <div className="min-h-0 flex-1">{ids.length > 0 && <ConnectionsMap key={ids.join(',')} investigationIds={ids} />}</div>
    </div>
  );
}
