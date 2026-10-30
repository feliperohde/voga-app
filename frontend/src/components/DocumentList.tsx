'use client';

import { useRouter } from 'next/navigation';
import { AudioLines, Globe, IdCard, Landmark, Mic, Newspaper, PlaySquare, Receipt, ScrollText, Vote, FileText, type LucideIcon, MessageCircle, HandCoins, BookOpen, FileSignature, Gavel } from 'lucide-react';
import { formatDate, useI18n } from '@/i18n';
import { RECORD_KINDS, type DocumentKind, type DocumentSummary } from '@/services/investigationService';

export const KIND_ICONS: Record<DocumentKind, LucideIcon> = {
  article: Newspaper,
  page: Globe,
  video: PlaySquare,
  live: AudioLines,
  speech: Mic,
  bill: ScrollText,
  vote: Vote,
  expense: Receipt,
  profile: IdCard,
  candidacy: Landmark,
  proposal: FileText,
  post: MessageCircle,
  amendment: HandCoins,
  gazette: BookOpen,
  contract: FileSignature,
  procurement: Gavel,
};

/** The date a document is shown with: when it was published, else when it was collected. */
export function documentDate(locale: Parameters<typeof formatDate>[0], document: Pick<DocumentSummary, 'publishedAt' | 'fetchedAt'>) {
  return formatDate(locale, document.publishedAt ?? document.fetchedAt, { dateStyle: 'medium' });
}

/** `investigation`: whose it is, on a list of several investigations. */
export function DocumentRow({ document, investigation }: { document: DocumentSummary; investigation?: string }) {
  const { t, locale } = useI18n();
  const router = useRouter();
  const Icon = KIND_ICONS[document.kind];
  return (
    <button
      onClick={() => router.push(`/document?id=${encodeURIComponent(document.id)}`)}
      className="flex w-full items-start gap-3 rounded-md px-3 py-2.5 text-left hover:bg-gray-50"
    >
      <Icon className="mt-0.5 h-4 w-4 flex-shrink-0 text-gray-500" />
      <span className="min-w-0 flex-1">
        <span className="block truncate font-medium text-gray-900">{document.title}</span>
        <span className="block truncate text-xs text-gray-500">
          {RECORD_KINDS.includes(document.kind) && (
            <span title={t('documents.recordHelp')} className="mr-1.5 rounded bg-blue-50 px-1 py-px text-[10px] font-medium uppercase text-blue-800">
              {t('documents.record')}
            </span>
          )}
          {[document.site, investigation, documentDate(locale, document)].filter(Boolean).join(' · ')}
          {document.note && <span className="ml-2 text-amber-700">· {t('documents.incomplete')}</span>}
        </span>
      </span>
    </button>
  );
}

/** `names`: investigation id → name, to say whose each document is (several open together). */
export function DocumentList({ documents, empty, names }: { documents: DocumentSummary[]; empty: string; names?: Record<string, string> }) {
  if (documents.length === 0) return <p className="px-3 py-6 text-center text-sm text-gray-500">{empty}</p>;
  return (
    <div className="divide-y divide-gray-100">
      {documents.map(document => (
        <DocumentRow key={document.id} document={document} investigation={names?.[document.investigationId]} />
      ))}
    </div>
  );
}
