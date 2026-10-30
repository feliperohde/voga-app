'use client';

import { useCallback, useEffect, useState } from 'react';
import { useRouter } from 'next/navigation';
import { AlertTriangle, ArrowRight, Scale } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { PageHeader } from '@/components/PageHeader';
import { ThinkingOrb } from '@/components/ThinkingOrb';
import { DocumentList } from '@/components/DocumentList';
import { AiSetup } from '@/components/AiSetup';
import { SourceSuggestions } from '@/components/SourceSuggestions';
import { ScandalsSection } from '@/components/ScandalsSection';
import { HighlightBlock, highlightHref, StatusBadge } from '@/components/home/InvestigationCard';
import { useInvestigation } from '@/contexts/InvestigationContext';
import { useCollectProgress } from '@/hooks/useCollectProgress';
import { useI18n } from '@/i18n';
import { DocumentSummary, investigationService, Source } from '@/services/investigationService';
import { claimsService, claimState } from '@/services/claimsService';
import { dashboardService, type InvestigationStatus } from '@/services/dashboardService';

/** One investigation's overview: its state, what to review and what came last. */
export default function InvestigationOverview() {
  const { t } = useI18n();
  const router = useRouter();
  const { current } = useInvestigation();
  const progress = useCollectProgress();
  const [documents, setDocuments] = useState<DocumentSummary[] | null>(null);
  const [total, setTotal] = useState(0);
  const [sources, setSources] = useState<Source[]>([]);
  const [toReview, setToReview] = useState(0);
  const [status, setStatus] = useState<InvestigationStatus | null>(null);

  const load = useCallback(async () => {
    if (!current) return;
    const [page, sources, claims, statuses] = await Promise.all([
      investigationService.documents([current.id], { limit: 12 }),
      investigationService.sources(current.id),
      claimsService.list(current.id),
      dashboardService.load(),
    ]);
    setStatus(statuses.find(s => s.id === current.id) ?? null);
    setDocuments(page.items);
    setTotal(page.total);
    setSources(sources);
    setToReview(claims.filter(c => claimState(c) === 'review').length);
  }, [current]);

  useEffect(() => {
    load().catch(error => console.error('Failed to load the investigation:', error));
  }, [load]);
  // New documents arrive while collecting
  useEffect(() => {
    if (progress?.collected) load().catch(() => {});
  }, [progress?.collected, load]);

  if (!current) return null;
  const failing = sources.filter(s => s.lastError && s.status !== 'paused' && s.status !== 'rejected');
  const suggested = sources.filter(s => s.status === 'suggested');

  return (
    <div>
      <PageHeader title={current.name} actions={status && <StatusBadge status={status} run={progress?.runs.find(r => r.investigationId === current.id)} />} />
      <div className="mx-auto max-w-4xl space-y-6 px-4 py-6 sm:px-8">
        {current.description && <p className="text-gray-600">{current.description}</p>}
        <AiSetup />
        {status && current.kind === 'politics' && <HighlightBlock highlight={status.latest} onOpen={() => status.latest && router.push(highlightHref(status.latest))} />}

        <div className="grid gap-3 sm:grid-cols-3">
          <div className="rounded-lg border border-gray-200 bg-white p-4 shadow-sm">
            <div className="text-2xl font-semibold text-gray-900">{total}</div>
            <div className="text-sm text-gray-500">{t('home.documents')}</div>
          </div>
          <div className="rounded-lg border border-gray-200 bg-white p-4 shadow-sm">
            <div className="text-2xl font-semibold text-gray-900">{sources.length}</div>
            <div className="text-sm text-gray-500">{t('home.sources')}</div>
          </div>
          <div className="rounded-lg border border-gray-200 bg-white p-4 shadow-sm">
            {progress?.running && progress.investigationId === current.id ? (
              <div className="flex items-center gap-2">
                <ThinkingOrb variant="background" size={22} />
                <div className="min-w-0">
                  <div className="text-sm font-medium text-gray-900">{t('collect.running')}</div>
                  <div className="truncate text-xs text-gray-500">{progress.current ?? t('collect.runningQueued', { count: progress.queued })}</div>
                </div>
              </div>
            ) : (
              <>
                <div className="text-2xl font-semibold text-gray-900">{status?.queued ?? 0}</div>
                <div className="text-sm text-gray-500">{t('home.queued')}</div>
              </>
            )}
          </div>
        </div>

        {sources.length === 0 && (
          <div className="flex items-center justify-between gap-4 rounded-lg border border-blue-200 bg-blue-50 p-4">
            <p className="text-sm text-blue-900">{t('home.noSources')}</p>
            <Button size="sm" onClick={() => router.push('/sources')}>
              {t('home.addSources')} <ArrowRight className="ml-2 h-4 w-4" />
            </Button>
          </div>
        )}

        <SourceSuggestions sources={suggested} onChanged={() => load().catch(() => {})} />

        {toReview > 0 && (
          <button
            onClick={() => router.push('/claims')}
            className="flex w-full items-center gap-3 rounded-lg border border-amber-200 bg-amber-50 p-4 text-left text-sm text-amber-900 hover:bg-amber-100"
          >
            <Scale className="h-4 w-4 flex-shrink-0" />
            {t('home.toReview', { count: toReview })}
            <ArrowRight className="ml-auto h-4 w-4" />
          </button>
        )}

        {failing.length > 0 && (
          <button
            onClick={() => router.push('/sources')}
            className="flex w-full items-center gap-3 rounded-lg border border-red-200 bg-red-50 p-4 text-left text-sm text-red-900 hover:bg-red-100"
          >
            <AlertTriangle className="h-4 w-4 flex-shrink-0" />
            {t('home.failingSources', { count: failing.length })}
          </button>
        )}

        {current.kind === 'politics' && <ScandalsSection investigationId={current.id} onChanged={() => load().catch(() => {})} />}

        <section className="rounded-lg border border-gray-200 bg-white shadow-sm">
          <div className="flex items-center justify-between border-b border-gray-100 px-4 py-3">
            <h2 className="font-semibold text-gray-900">{t('home.latest')}</h2>
            {total > 0 && (
              <Button variant="ghost" size="sm" onClick={() => router.push('/documents')}>
                {t('home.allDocuments')} <ArrowRight className="ml-1 h-4 w-4" />
              </Button>
            )}
          </div>
          {documents === null ? (
            <div className="flex justify-center p-6">
              <ThinkingOrb variant="working" size={20} />
            </div>
          ) : (
            <DocumentList documents={documents} empty={t('home.noDocuments')} />
          )}
        </section>
      </div>
    </div>
  );
}
