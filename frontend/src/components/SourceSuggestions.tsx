'use client';

import { Check, Newspaper, ScanSearch, Search, Sparkles, X } from 'lucide-react';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import { useInvestigation } from '@/contexts/InvestigationContext';
import { MessageKey, useI18n } from '@/i18n';
import { discoveryService, Source } from '@/services/investigationService';

const ICONS = { news_search: Newspaper, youtube_search: Search, web_search: ScanSearch } as const;

/** The searches the AI suggested, waiting for a person to approve or dismiss them. */
export function SourceSuggestions({ sources, onChanged }: { sources: Source[]; onChanged: () => void }) {
  const { t } = useI18n();
  const { current } = useInvestigation();
  if (sources.length === 0) return null;

  const act = (action: Promise<void>, done: string) =>
    action
      .then(() => {
        toast.success(done);
        onChanged();
      })
      .catch(error => toast.error(String(error)));

  return (
    <section className="rounded-lg border border-violet-200 bg-violet-50/60 shadow-sm">
      <div className="flex items-start gap-3 border-b border-violet-100 px-5 py-3">
        <Sparkles className="mt-0.5 h-4 w-4 flex-shrink-0 text-violet-700" />
        <div>
          <h2 className="font-semibold text-gray-900">{t('suggestions.title', { count: sources.length })}</h2>
          <p className="text-sm text-gray-600">{t(current?.kind === 'interest' ? 'suggestions.help.interest' : 'suggestions.help')}</p>
        </div>
      </div>
      <div className="divide-y divide-violet-100">
        {sources.map(source => {
          const Icon = ICONS[source.kind as keyof typeof ICONS] ?? Search;
          return (
            <div key={source.id} className="flex items-start gap-3 px-5 py-3">
              <Icon className="mt-0.5 h-4 w-4 flex-shrink-0 text-gray-500" />
              <div className="min-w-0 flex-1">
                <div className="font-medium text-gray-900">
                  {t(`sources.kind.${source.kind}` as MessageKey)}: {source.url}
                </div>
                {source.note && <div className="text-xs text-gray-600">{source.note}</div>}
              </div>
              <div className="flex flex-shrink-0 gap-1">
                <Button size="sm" variant="outline" onClick={() => act(discoveryService.approve(source.id), t('suggestions.approved'))}>
                  <Check className="mr-1.5 h-4 w-4" /> {t('suggestions.approve')}
                </Button>
                <Button size="icon" variant="ghost" title={t('suggestions.reject')} onClick={() => act(discoveryService.reject(source.id), t('suggestions.rejected'))}>
                  <X className="h-4 w-4" />
                </Button>
              </div>
            </div>
          );
        })}
      </div>
    </section>
  );
}
