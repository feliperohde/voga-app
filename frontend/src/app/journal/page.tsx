'use client';

import { Suspense, useCallback, useEffect, useRef, useState } from 'react';
import { useRouter, useSearchParams } from 'next/navigation';
import { CheckCheck, MoreHorizontal, Radio, Star, ThumbsDown, ThumbsUp, Trash2 } from 'lucide-react';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from '@/components/ui/dropdown-menu';
import { SelectField } from '@/components/ui/select';
import { Switch } from '@/components/ui/switch';
import { Label } from '@/components/ui/label';
import { PageHeader } from '@/components/PageHeader';
import { ThinkingOrb } from '@/components/ThinkingOrb';
import { NewsCard } from '@/components/home/NewsCard';
import { useInvestigation } from '@/contexts/InvestigationContext';
import { useCollectProgress } from '@/hooks/useCollectProgress';
import { useDeleteInvestigation } from '@/hooks/useDeleteInvestigation';
import { useI18n } from '@/i18n';
import { feedService, type FeedItem } from '@/services/feedService';
import type { Feedback } from '@/services/investigationService';
import { cn } from '@/lib/utils';

const PAGE_SIZE = 24;
const WEEK = 7 * 86400000;

type Sort = 'date' | 'relevance';

/** The Journal: what every interest collected, as cards with their pictures. */
function Journal() {
  const { t } = useI18n();
  const router = useRouter();
  const params = useSearchParams();
  const { investigations, context, open } = useInvestigation();
  const progress = useCollectProgress();
  const remove = useDeleteInvestigation();
  const interestId = params.get('id');
  const starredOnly = params.get('starred') === '1';
  const [sort, setSort] = useState<Sort>('date');
  const [unreadOnly, setUnreadOnly] = useState(false);
  const [items, setItems] = useState<FeedItem[] | null>(null);
  const [total, setTotal] = useState(0);
  const interests = investigations.filter(i => i.kind === 'interest');
  const interest = interests.find(i => i.id === interestId);

  // Leaving for a subject's own pages: the Journal does not take the sidebar back on its way out
  const leaving = useRef(false);
  // Opened by its address (or from another context): the sidebar is the Journal's
  useEffect(() => {
    if (!leaving.current && context?.kind !== 'journal') open({ kind: 'journal' });
  }, [context, open]);

  const load = useCallback(
    async (offset = 0) => {
      const page = await feedService.list({
        investigationIds: interestId ? [interestId] : null,
        investigationKind: 'interest',
        sort,
        // Most relevant of the week; newest of all time
        since: sort === 'relevance' ? new Date(Date.now() - WEEK).toISOString() : null,
        unreadOnly,
        starredOnly,
        limit: PAGE_SIZE,
        offset,
      });
      setItems(previous => (offset === 0 ? page.items : [...(previous ?? []), ...page.items]));
      setTotal(page.total);
    },
    [interestId, sort, unreadOnly, starredOnly],
  );

  useEffect(() => {
    setItems(null);
    load().catch(error => console.error('Failed to load the Journal:', error));
  }, [load]);
  // What a collection run brought
  useEffect(() => {
    if (progress && !progress.running) load().catch(() => {});
  }, [progress?.running, progress, load]);

  const update = (id: string, change: Partial<FeedItem>) => setItems(previous => previous && previous.map(item => (item.id === id ? { ...item, ...change } : item)));

  const openItem = (item: FeedItem) => {
    update(item.id, { read: true });
    feedService.markRead([item.id]).catch(() => {});
    router.push(`/document?id=${encodeURIComponent(item.id)}`);
  };

  const setFlags = (item: FeedItem, flags: { starred?: boolean; feedback?: Feedback }) => {
    update(item.id, flags);
    feedService.setFlags(item.id, flags).catch(error => {
      update(item.id, { starred: item.starred, feedback: item.feedback });
      toast.error(t('common.saveFailed'), { description: String(error) });
    });
  };

  const markAllRead = () => {
    feedService
      .markAllRead(interestId ? [interestId] : null, 'interest')
      .then(() => load())
      .catch(error => toast.error(t('common.saveFailed'), { description: String(error) }));
  };

  const filter = (id: string | null) => router.replace(id ? `/journal?id=${encodeURIComponent(id)}` : '/journal');

  // The subject shown, opened alone: its themes, sources and schedule
  const openSources = (id: string) => {
    leaving.current = true;
    open({ kind: 'investigation', ids: [id] });
    router.push('/sources');
  };
  const deleteInterest = async (id: string) => {
    const shown = interests.find(i => i.id === id);
    if (shown && (await remove({ id, name: shown.name, kind: 'interest', documents: shown.documentCount }))) filter(null);
  };

  return (
    <div>
      <PageHeader
        title={starredOnly ? t('nav.starred') : (interest?.name ?? t('nav.journal'))}
        actions={
          <div className="flex items-center gap-1">
            <Button variant="outline" size="sm" onClick={markAllRead}>
              <CheckCheck className="mr-2 h-4 w-4" /> {t('feed.markAllRead')}
            </Button>
            {interest && (
              <DropdownMenu>
                <DropdownMenuTrigger asChild>
                  <Button variant="ghost" size="icon" title={t('common.moreOptions')} aria-label={t('common.moreOptionsOf', { name: interest.name })}>
                    <MoreHorizontal className="h-4 w-4" />
                  </Button>
                </DropdownMenuTrigger>
                <DropdownMenuContent align="end">
                  <DropdownMenuItem onSelect={() => openSources(interest.id)}>
                    <Radio className="mr-2 h-4 w-4" /> {t('journal.sources')}
                  </DropdownMenuItem>
                  <DropdownMenuItem onSelect={() => deleteInterest(interest.id).catch(() => {})} className="text-red-700 focus:text-red-700">
                    <Trash2 className="mr-2 h-4 w-4" /> {t('interest.delete')}
                  </DropdownMenuItem>
                </DropdownMenuContent>
              </DropdownMenu>
            )}
          </div>
        }
      />
      <div className="mx-auto max-w-6xl space-y-4 px-4 py-6 sm:px-8">
        {!starredOnly && interests.length > 1 && (
          <div className="flex flex-wrap gap-2">
            {[{ id: null, name: t('journal.all') }, ...interests].map(option => (
              <button
                key={option.id ?? 'all'}
                onClick={() => filter(option.id)}
                aria-pressed={option.id === interestId}
                className={cn(
                  'rounded-full border px-3 py-1 text-sm transition',
                  option.id === interestId ? 'border-gray-900 bg-gray-900 text-white' : 'border-gray-200 bg-white text-gray-700 hover:bg-gray-50',
                )}
              >
                {option.name}
              </button>
            ))}
          </div>
        )}
        <div className="flex flex-wrap items-center gap-3">
          <div className="w-40">
            <SelectField
              value={sort}
              onValueChange={value => setSort(value as Sort)}
              aria-label={t('feed.sort')}
              options={[
                { value: 'date', label: t('feed.sort.date') },
                { value: 'relevance', label: t('feed.sort.relevance') },
              ]}
            />
          </div>
          <div className="flex items-center gap-2">
            <Switch id="journal-unread" checked={unreadOnly} onCheckedChange={setUnreadOnly} />
            <Label htmlFor="journal-unread" className="text-sm font-normal">
              {t('feed.unreadOnly')}
            </Label>
          </div>
        </div>

        {items === null ? (
          <div className="flex justify-center p-6">
            <ThinkingOrb variant="working" size={20} />
          </div>
        ) : items.length === 0 ? (
          <p className="rounded-lg border border-gray-200 bg-white px-4 py-8 text-center text-sm text-gray-500 shadow-sm">
            {t(interests.length === 0 ? 'journal.noInterests' : sort === 'relevance' ? 'feed.emptyWeek' : 'journal.empty')}
          </p>
        ) : (
          <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
            {items.map((item, i) => (
              <NewsCard
                key={item.id}
                item={item}
                big={i === 0 && sort === 'date' && !starredOnly}
                onOpen={() => openItem(item)}
                actions={
                  <>
                    <Button variant="ghost" size="icon" className="h-8 w-8" title={t('feed.star')} aria-pressed={item.starred} onClick={() => setFlags(item, { starred: !item.starred })}>
                      <Star className={`h-4 w-4 ${item.starred ? 'fill-amber-400 text-amber-500' : ''}`} />
                    </Button>
                    <Button variant="ghost" size="icon" className="h-8 w-8" title={t('feed.more')} aria-pressed={item.feedback === 1} onClick={() => setFlags(item, { feedback: item.feedback === 1 ? 0 : 1 })}>
                      <ThumbsUp className={`h-4 w-4 ${item.feedback === 1 ? 'fill-blue-200 text-blue-600' : ''}`} />
                    </Button>
                    <Button variant="ghost" size="icon" className="h-8 w-8" title={t('feed.less')} aria-pressed={item.feedback === -1} onClick={() => setFlags(item, { feedback: item.feedback === -1 ? 0 : -1 })}>
                      <ThumbsDown className={`h-4 w-4 ${item.feedback === -1 ? 'fill-gray-300 text-gray-700' : ''}`} />
                    </Button>
                  </>
                }
              />
            ))}
          </div>
        )}
        {items && items.length < total && (
          <div className="flex justify-center">
            <Button variant="ghost" size="sm" onClick={() => load(items.length).catch(() => {})}>
              {t('documents.more', { count: total - items.length })}
            </Button>
          </div>
        )}
      </div>
    </div>
  );
}

export default function JournalPage() {
  return (
    <Suspense fallback={null}>
      <Journal />
    </Suspense>
  );
}
