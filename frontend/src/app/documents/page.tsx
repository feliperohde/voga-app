'use client';

import { useCallback, useEffect, useState } from 'react';
import { useRouter } from 'next/navigation';
import { Search, X } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { SelectField } from '@/components/ui/select';
import { PageHeader } from '@/components/PageHeader';
import { ThinkingOrb } from '@/components/ThinkingOrb';
import { DocumentList, KIND_ICONS } from '@/components/DocumentList';
import { useInvestigation } from '@/contexts/InvestigationContext';
import { formatDate, useI18n } from '@/i18n';
import { askService, SearchResponse } from '@/services/askService';
import { DOCUMENT_KINDS, DocumentKind, DocumentSummary, formatClock, investigationService } from '@/services/investigationService';

const PAGE_SIZE = 50;

function SearchResults({ response }: { response: SearchResponse }) {
  const { t, locale } = useI18n();
  const router = useRouter();
  if (response.results.length === 0) return <p className="px-3 py-6 text-center text-sm text-gray-500">{t('documents.noResults')}</p>;
  return (
    <div className="divide-y divide-gray-100">
      {response.vectorError && <p className="px-3 py-2 text-xs text-amber-700">{t('documents.keywordsOnly')}</p>}
      {response.results.map(hit => {
        const Icon = KIND_ICONS[hit.kind];
        const at = hit.startTime != null ? `&t=${Math.floor(hit.startTime)}` : '';
        return (
          <button
            key={hit.chunkId}
            onClick={() => router.push(`/document?id=${encodeURIComponent(hit.documentId)}${at}`)}
            className="flex w-full items-start gap-3 px-3 py-3 text-left hover:bg-gray-50"
          >
            <Icon className="mt-0.5 h-4 w-4 flex-shrink-0 text-gray-500" />
            <span className="min-w-0 flex-1">
              <span className="block truncate font-medium text-gray-900">{hit.title}</span>
              <span className="block text-xs text-gray-500">
                {[hit.site, formatDate(locale, hit.docDate, { dateStyle: 'medium' }), hit.startTime != null && formatClock(hit.startTime)].filter(Boolean).join(' · ')}
              </span>
              <span className="mt-1 line-clamp-3 block text-sm text-gray-700">{hit.text}</span>
            </span>
          </button>
        );
      })}
    </div>
  );
}

export default function DocumentsPage() {
  const { t } = useI18n();
  const { scope } = useInvestigation();
  const ids = scope.map(i => i.id).join(',');
  // Several open together: each document says whose it is
  const names = scope.length > 1 ? Object.fromEntries(scope.map(i => [i.id, i.name])) : undefined;
  const [kind, setKind] = useState<DocumentKind | 'all'>('all');
  const [documents, setDocuments] = useState<DocumentSummary[] | null>(null);
  const [total, setTotal] = useState(0);
  const [query, setQuery] = useState('');
  const [searching, setSearching] = useState(false);
  const [results, setResults] = useState<SearchResponse | null>(null);

  const load = useCallback(
    async (offset = 0) => {
      if (!ids) return;
      const page = await investigationService.documents(ids.split(','), { kind: kind === 'all' ? undefined : kind, limit: PAGE_SIZE, offset });
      setDocuments(previous => (offset === 0 ? page.items : [...(previous ?? []), ...page.items]));
      setTotal(page.total);
    },
    [ids, kind],
  );

  useEffect(() => {
    setDocuments(null);
    load().catch(error => console.error('Failed to load documents:', error));
  }, [load]);

  const search = async () => {
    if (!ids || !query.trim()) return;
    setSearching(true);
    try {
      setResults(await askService.search(ids.split(','), query.trim()));
    } catch (error) {
      console.error('Search failed:', error);
    } finally {
      setSearching(false);
    }
  };

  if (!ids) return <PageHeader title={t('nav.documents')} />;

  return (
    <div>
      <PageHeader title={t('nav.documents')} />
      <div className="mx-auto max-w-4xl space-y-4 px-4 py-6 sm:px-8">
        <div className="flex gap-2">
          <div className="relative flex-1">
            <Search className="absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-gray-400" />
            <Input
              className="pl-9"
              value={query}
              onChange={e => {
                setQuery(e.target.value);
                if (!e.target.value) setResults(null);
              }}
              onKeyDown={e => e.key === 'Enter' && search()}
              placeholder={t('documents.searchPlaceholder')}
            />
          </div>
          {results ? (
            <Button
              variant="outline"
              onClick={() => {
                setQuery('');
                setResults(null);
              }}
            >
              <X className="mr-2 h-4 w-4" /> {t('documents.clearSearch')}
            </Button>
          ) : (
            <Button onClick={search} disabled={searching || !query.trim()}>
              {searching && <ThinkingOrb variant="working" size={16} className="mr-2" />}
              {t('documents.search')}
            </Button>
          )}
        </div>

        <section className="rounded-lg border border-gray-200 bg-white shadow-sm">
          {results ? (
            <SearchResults response={results} />
          ) : (
            <>
              <div className="flex items-center justify-between gap-3 border-b border-gray-100 px-3 py-2">
                <span className="text-sm text-gray-500">{t('documents.count', { count: total })}</span>
                <div className="w-56">
                  <SelectField
                    value={kind}
                    onValueChange={value => setKind(value as DocumentKind | 'all')}
                    aria-label={t('documents.kindFilter')}
                    options={[{ value: 'all', label: t('documents.all') }, ...DOCUMENT_KINDS.map(k => ({ value: k, label: t(`documents.kind.${k}`) }))]}
                  />
                </div>
              </div>
              {documents === null ? (
                <div className="flex justify-center p-6">
                  <ThinkingOrb variant="working" size={20} />
                </div>
              ) : (
                <DocumentList documents={documents} empty={t('documents.empty')} names={names} />
              )}
              {documents && documents.length < total && (
                <div className="flex justify-center border-t border-gray-100 p-3">
                  <Button variant="ghost" size="sm" onClick={() => load(documents.length).catch(() => {})}>
                    {t('documents.more', { count: total - documents.length })}
                  </Button>
                </div>
              )}
            </>
          )}
        </section>
      </div>
    </div>
  );
}
