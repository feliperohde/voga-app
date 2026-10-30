'use client';

import { Suspense, useEffect, useRef, useState } from 'react';
import { useRouter, useSearchParams } from 'next/navigation';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { ExternalLink, Siren, Star, ThumbsDown, ThumbsUp, Trash2 } from 'lucide-react';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import { useConfirm } from '@/components/ui/confirm-dialog';
import { PageHeader } from '@/components/PageHeader';
import { ThinkingOrb } from '@/components/ThinkingOrb';
import { ScandalDialog } from '@/components/ScandalDialog';
import { useInvestigation } from '@/contexts/InvestigationContext';
import { documentDate, KIND_ICONS } from '@/components/DocumentList';
import { useI18n } from '@/i18n';
import { openExternal } from '@/lib/openExternal';
import { feedService } from '@/services/feedService';
import { documentLink, Feedback, formatClock, investigationService, VogaDocument } from '@/services/investigationService';
import { liveService, sessionOfDocument } from '@/services/liveService';

function Subtitles({ document, at }: { document: VogaDocument; at: number | null }) {
  const { t } = useI18n();
  const target = useRef<HTMLButtonElement | null>(null);
  const segments = document.segments ?? [];
  // A live session's transcript: a line plays what was said
  const session = sessionOfDocument(document.url);
  const open = (start: number, end: number) =>
    session
      ? liveService.playClip(session, start, end).catch(error => toast.error(t('live.playFailed'), { description: String(error) }))
      : openExternal(documentLink(document.url, start));
  // The line the citation points at: the last one starting at or before it
  const focused = at == null ? -1 : segments.reduce((best, s, i) => (s.start <= at + 0.5 ? i : best), -1);
  useEffect(() => {
    target.current?.scrollIntoView({ block: 'center' });
  }, [focused]);
  return (
    <div className="space-y-0.5">
      <p className="mb-3 text-xs text-gray-500">{t(session ? 'live.document.help' : 'document.subtitlesHelp')}</p>
      {segments.map((segment, i) => (
        <button
          key={i}
          ref={i === focused ? target : undefined}
          onClick={() => open(segment.start, segment.end)}
          className={`flex w-full gap-3 rounded px-2 py-1 text-left text-sm hover:bg-gray-50 ${i === focused ? 'bg-amber-50 ring-1 ring-amber-200' : ''}`}
        >
          <span className="w-14 flex-shrink-0 font-mono text-xs leading-5 text-gray-400">{formatClock(segment.start)}</span>
          <span className="select-text text-gray-800">{segment.text}</span>
        </button>
      ))}
    </div>
  );
}

function DocumentView() {
  const { t, locale } = useI18n();
  const router = useRouter();
  const confirm = useConfirm();
  const params = useSearchParams();
  const id = params.get('id');
  const at = params.get('t') ? Number(params.get('t')) : null;
  const [document, setDocument] = useState<VogaDocument | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [marking, setMarking] = useState(false);
  const { investigations } = useInvestigation();

  useEffect(() => {
    if (!id) return;
    investigationService.document(id).then(setDocument).catch(e => setError(String(e)));
    feedService.markRead([id]).catch(() => {});
  }, [id]);

  const setFlags = (flags: { starred?: boolean; feedback?: Feedback }) => {
    if (!document) return;
    const before = document;
    setDocument({ ...document, ...flags });
    feedService.setFlags(document.id, flags).catch(error => {
      setDocument(before);
      toast.error(t('common.saveFailed'), { description: String(error) });
    });
  };

  const remove = async () => {
    if (!document) return;
    if (await confirm({ title: t('document.deleteTitle'), description: t('document.deleteHelp'), destructive: true, confirmLabel: t('common.delete') })) {
      investigationService
        .deleteDocument(document.id)
        .then(() => router.back())
        .catch(e => toast.error(String(e)));
    }
  };

  if (error) return <p className="p-8 text-sm text-red-600">{error}</p>;
  if (!document) {
    return (
      <div className="flex min-h-[60vh] items-center justify-center">
        <ThinkingOrb variant="working" size={28} />
      </div>
    );
  }
  const Icon = KIND_ICONS[document.kind];
  // News about people followed can be marked a scandal
  const politics = investigations.find(i => i.id === document.investigationId)?.kind === 'politics';
  const meta = [document.site, document.author !== document.site && document.author, documentDate(locale, document)].filter(Boolean);

  return (
    <div>
      <PageHeader
        onBack={() => router.back()}
        actions={
          <>
            <Button variant="ghost" size="icon" title={t('feed.star')} aria-pressed={document.starred} onClick={() => setFlags({ starred: !document.starred })}>
              <Star className={`h-4 w-4 ${document.starred ? 'fill-amber-400 text-amber-500' : ''}`} />
            </Button>
            <Button variant="ghost" size="icon" title={t('feed.more')} aria-pressed={document.feedback === 1} onClick={() => setFlags({ feedback: document.feedback === 1 ? 0 : 1 })}>
              <ThumbsUp className={`h-4 w-4 ${document.feedback === 1 ? 'fill-blue-200 text-blue-600' : ''}`} />
            </Button>
            <Button variant="ghost" size="icon" title={t('feed.less')} aria-pressed={document.feedback === -1} onClick={() => setFlags({ feedback: document.feedback === -1 ? 0 : -1 })}>
              <ThumbsDown className={`h-4 w-4 ${document.feedback === -1 ? 'fill-gray-300 text-gray-700' : ''}`} />
            </Button>
            {sessionOfDocument(document.url) ? (
              <Button variant="outline" size="sm" onClick={() => router.push(`/live?id=${encodeURIComponent(sessionOfDocument(document.url)!)}`)}>
                <ExternalLink className="mr-2 h-4 w-4" /> {t('live.openSession')}
              </Button>
            ) : (
              <Button variant="outline" size="sm" onClick={() => openExternal(document.url)}>
                <ExternalLink className="mr-2 h-4 w-4" /> {t('document.openOriginal')}
              </Button>
            )}
            {politics && (
              <Button variant="ghost" size="icon" title={t('scandal.mark')} onClick={() => setMarking(true)}>
                <Siren className="h-4 w-4" />
              </Button>
            )}
            <Button variant="ghost" size="icon" title={t('common.delete')} onClick={remove}>
              <Trash2 className="h-4 w-4" />
            </Button>
          </>
        }
      />
      {politics && <ScandalDialog document={document} open={marking} onOpenChange={setMarking} />}
      <article className="mx-auto max-w-3xl space-y-4 px-4 py-6 sm:px-8">
        {/* The headline belongs to the text: it wraps here instead of being cut in the header */}
        <h1 className="select-text break-words text-3xl font-bold leading-tight text-gray-900">{document.title}</h1>
        <div className="flex items-center gap-2 text-sm text-gray-500">
          <Icon className="h-4 w-4" />
          <span>{meta.join(' · ')}</span>
        </div>
        <p className="break-all text-xs text-gray-400">{document.url}</p>
        {document.note && <p className="rounded-md border border-amber-200 bg-amber-50 p-3 text-sm text-amber-900">{document.note}</p>}
        {document.indexError && <p className="text-xs text-amber-700">{t('document.keywordsOnly')}</p>}
        {document.segments?.length ? (
          <Subtitles document={document} at={at} />
        ) : (
          <div className="prose prose-sm max-w-none select-text dark:prose-invert">
            <ReactMarkdown remarkPlugins={[remarkGfm]}>{document.text}</ReactMarkdown>
          </div>
        )}
      </article>
    </div>
  );
}

export default function DocumentPage() {
  return (
    <Suspense fallback={null}>
      <DocumentView />
    </Suspense>
  );
}
