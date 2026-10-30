'use client';

import { useRouter } from 'next/navigation';
import { ExternalLink, FileText } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { KIND_ICONS } from '@/components/DocumentList';
import { formatDate, useI18n } from '@/i18n';
import { openExternal } from '@/lib/openExternal';
import type { Citation } from '@/services/askService';
import { documentLink, formatClock } from '@/services/investigationService';

/** A passage an AI answer cites as [n], with the way to its document and to the original. */
export function CitationCard({ citation }: { citation: Citation }) {
  const { t, locale } = useI18n();
  const router = useRouter();
  const Icon = KIND_ICONS[citation.kind];
  const at = citation.startTime != null ? `&t=${Math.floor(citation.startTime)}` : '';
  return (
    <div className="rounded-md border border-gray-200 bg-white p-3 text-sm">
      <div className="flex items-start gap-2">
        <span className="mt-0.5 flex h-5 min-w-5 items-center justify-center rounded bg-gray-100 px-1 text-xs font-semibold text-gray-700">{citation.index}</span>
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-1.5 font-medium text-gray-900">
            <Icon className="h-3.5 w-3.5 flex-shrink-0 text-gray-500" />
            <span className="truncate">{citation.title}</span>
          </div>
          <div className="text-xs text-gray-500">
            {[citation.site, formatDate(locale, citation.docDate, { dateStyle: 'medium' }), citation.startTime != null && formatClock(citation.startTime)]
              .filter(Boolean)
              .join(' · ')}
          </div>
          <p className="mt-1.5 line-clamp-4 select-text text-gray-700">{citation.excerpt}</p>
          <div className="mt-2 flex gap-1">
            <Button variant="ghost" size="sm" className="h-7 px-2 text-xs" onClick={() => router.push(`/document?id=${encodeURIComponent(citation.documentId)}${at}`)}>
              <FileText className="mr-1 h-3.5 w-3.5" /> {t('ask.openDocument')}
            </Button>
            <Button variant="ghost" size="sm" className="h-7 px-2 text-xs" onClick={() => openExternal(documentLink(citation.url, citation.startTime))}>
              <ExternalLink className="mr-1 h-3.5 w-3.5" /> {t('ask.openOriginal')}
            </Button>
          </div>
        </div>
      </div>
    </div>
  );
}
