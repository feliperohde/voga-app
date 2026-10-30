'use client';

import { useCallback, useEffect, useState } from 'react';
import { useRouter } from 'next/navigation';
import { CheckCheck, Star, ThumbsDown, ThumbsUp } from 'lucide-react';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import { SelectField } from '@/components/ui/select';
import { Switch } from '@/components/ui/switch';
import { Label } from '@/components/ui/label';
import { PageHeader } from '@/components/PageHeader';
import { ThinkingOrb } from '@/components/ThinkingOrb';
import { KIND_ICONS } from '@/components/DocumentList';
import { useInvestigation } from '@/contexts/InvestigationContext';
import { useCollectProgress } from '@/hooks/useCollectProgress';
import { formatDate, useI18n } from '@/i18n';
import { feedService, type FeedItem } from '@/services/feedService';
import type { Feedback } from '@/services/investigationService';
import { plainText } from '@/lib/utils';

const PAGE_SIZE = 50;
const DAY = 86400000;

type Sort = 'date' | 'relevance';

const itemDate = (item: FeedItem) => item.publishedAt ?? item.fetchedAt;

/** The day an item is listed under, where the user is. */
const dayKey = (value: string) => new Date(value).toLocaleDateString('en-CA');

function FeedRow({
  item,
  showInvestigation,
  spread,
  onOpen,
  onFlags,
}: {
  item: FeedItem;
  showInvestigation: boolean;
  /** The lowest and highest relevance listed: the bar is relative to them. */
  spread: [number, number];
  onOpen: () => void;
  onFlags: (flags: { starred?: boolean; feedback?: Feedback }) => void;
}) {
  const { t, locale } = useI18n();
  const Icon = KIND_ICONS[item.kind];
  const excerpt = plainText(item.excerpt);
  const [low, high] = spread;
  const share = item.relevance == null ? null : high > low ? (item.relevance - low) / (high - low) : 1;
  const meta = [item.site, showInvestigation && item.investigationName, formatDate(locale, itemDate(item), { dateStyle: 'short', timeStyle: 'short' })].filter(Boolean);
  return (
    <div className={`group flex items-start gap-3 px-4 py-3 hover:bg-gray-50 ${item.feedback === -1 ? 'opacity-60' : ''}`}>
      <span className={`mt-2 h-2 w-2 flex-shrink-0 rounded-full ${item.read ? 'bg-transparent' : 'bg-blue-500'}`} aria-label={item.read ? undefined : t('feed.unread')} />
      <button onClick={onOpen} className="min-w-0 flex-1 text-left">
        <span className={`block ${item.read ? 'text-gray-700' : 'font-semibold text-gray-900'}`}>{item.title}</span>
        {excerpt && <span className="mt-0.5 line-clamp-2 block text-sm text-gray-600">{excerpt}</span>}
        <span className="mt-1 flex items-center gap-2 text-xs text-gray-500">
          <Icon className="h-3.5 w-3.5 flex-shrink-0" />
          <span className="truncate">{meta.join(' · ')}</span>
          {share != null && (
            <span className="ml-1 inline-block h-1.5 w-12 flex-shrink-0 overflow-hidden rounded-full bg-gray-200" title={t('feed.relevance')}>
              <span className="block h-full rounded-full bg-blue-500" style={{ width: `${Math.max(8, share * 100)}%` }} />
            </span>
          )}
        </span>
      </button>
      <div className="flex flex-shrink-0 items-center gap-0.5">
        <Button variant="ghost" size="icon" className="h-8 w-8" title={t('feed.star')} aria-pressed={item.starred} onClick={() => onFlags({ starred: !item.starred })}>
          <Star className={`h-4 w-4 ${item.starred ? 'fill-amber-400 text-amber-500' : ''}`} />
        </Button>
        <Button variant="ghost" size="icon" className="h-8 w-8" title={t('feed.more')} aria-pressed={item.feedback === 1} onClick={() => onFlags({ feedback: item.feedback === 1 ? 0 : 1 })}>
          <ThumbsUp className={`h-4 w-4 ${item.feedback === 1 ? 'fill-blue-200 text-blue-600' : ''}`} />
        </Button>
        <Button variant="ghost" size="icon" className="h-8 w-8" title={t('feed.less')} aria-pressed={item.feedback === -1} onClick={() => onFlags({ feedback: item.feedback === -1 ? 0 : -1 })}>
          <ThumbsDown className={`h-4 w-4 ${item.feedback === -1 ? 'fill-gray-300 text-gray-700' : ''}`} />
        </Button>
      </div>
    </div>
  );
}

export default function FeedPage() {
  const { t, locale } = useI18n();
  const router = useRouter();
  const { scope } = useInvestigation();
  const progress = useCollectProgress();
  const [sort, setSort] = useState<Sort>('date');
  const [unreadOnly, setUnreadOnly] = useState(false);
  const [starredOnly, setStarredOnly] = useState(false);
  const [items, setItems] = useState<FeedItem[] | null>(null);
  const [highlights, setHighlights] = useState<FeedItem[]>([]);
  const [total, setTotal] = useState(0);

  const ids = scope.map(i => i.id).join(',');
  const load = useCallback(
    async (offset = 0) => {
      if (!ids) return;
      const investigationIds = ids.split(',');
      const page = await feedService.list({
        investigationIds,
        sort,
        // Most relevant of the week; newest of all time
        since: sort === 'relevance' ? new Date(Date.now() - 7 * DAY).toISOString() : null,
        unreadOnly,
        starredOnly,
        limit: PAGE_SIZE,
        offset,
      });
      setItems(previous => (offset === 0 ? page.items : [...(previous ?? []), ...page.items]));
      setTotal(page.total);
      if (offset === 0 && sort === 'date' && !starredOnly) {
        const top = await feedService.list({ investigationIds, sort: 'relevance', since: new Date(Date.now() - DAY).toISOString(), unreadOnly: true, limit: 5 });
        setHighlights(top.items.filter(item => item.relevance != null));
      } else if (offset === 0) {
        setHighlights([]);
      }
    },
    [ids, sort, unreadOnly, starredOnly],
  );

  useEffect(() => {
    setItems(null);
    load().catch(error => console.error('Failed to load the feed:', error));
  }, [load]);
  // What a collection run brought
  useEffect(() => {
    if (progress && !progress.running) load().catch(() => {});
  }, [progress?.running, progress, load]);

  const update = (id: string, change: Partial<FeedItem>) => {
    const apply = (list: FeedItem[]) => list.map(item => (item.id === id ? { ...item, ...change } : item));
    setItems(previous => previous && apply(previous));
    setHighlights(apply);
  };

  const open = (item: FeedItem) => {
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
      .markAllRead(ids.split(','))
      .then(() => load())
      .catch(error => toast.error(t('common.saveFailed'), { description: String(error) }));
  };

  const scored = [...(items ?? []), ...highlights].map(item => item.relevance).filter((r): r is number => r != null);
  const spread: [number, number] = [Math.min(...scored), Math.max(...scored)];
  const row = (item: FeedItem) => (
    <FeedRow key={item.id} item={item} showInvestigation={scope.length > 1} spread={spread} onOpen={() => open(item)} onFlags={flags => setFlags(item, flags)} />
  );

  // Newest first, under the day they came out
  const days: { key: string; label: string; items: FeedItem[] }[] = [];
  if (sort === 'date') {
    const today = dayKey(new Date().toISOString());
    const yesterday = dayKey(new Date(Date.now() - DAY).toISOString());
    for (const item of items ?? []) {
      const key = dayKey(itemDate(item));
      if (days[days.length - 1]?.key !== key) {
        const label = key === today ? t('feed.today') : key === yesterday ? t('feed.yesterday') : formatDate(locale, itemDate(item), { dateStyle: 'full' });
        days.push({ key, label, items: [] });
      }
      days[days.length - 1].items.push(item);
    }
  }

  return (
    <div>
      <PageHeader
        title={t('nav.news')}
        actions={
          <Button variant="outline" size="sm" onClick={markAllRead}>
            <CheckCheck className="mr-2 h-4 w-4" /> {t('feed.markAllRead')}
          </Button>
        }
      />
      <div className="mx-auto max-w-4xl space-y-4 px-4 py-6 sm:px-8">
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
            <Switch id="feed-unread" checked={unreadOnly} onCheckedChange={setUnreadOnly} />
            <Label htmlFor="feed-unread" className="text-sm font-normal">
              {t('feed.unreadOnly')}
            </Label>
          </div>
          <div className="flex items-center gap-2">
            <Switch id="feed-starred" checked={starredOnly} onCheckedChange={setStarredOnly} />
            <Label htmlFor="feed-starred" className="text-sm font-normal">
              {t('feed.starredOnly')}
            </Label>
          </div>
        </div>

        {highlights.length > 0 && (
          <section className="rounded-lg border border-blue-100 bg-blue-50/40 shadow-sm">
            <h2 className="px-4 pt-3 text-sm font-semibold text-gray-900">{t('feed.highlights')}</h2>
            <p className="px-4 text-xs text-gray-500">{t('feed.highlightsHelp')}</p>
            <div className="divide-y divide-blue-100">{highlights.map(row)}</div>
          </section>
        )}

        <section className="rounded-lg border border-gray-200 bg-white shadow-sm">
          {items === null ? (
            <div className="flex justify-center p-6">
              <ThinkingOrb variant="working" size={20} />
            </div>
          ) : items.length === 0 ? (
            <p className="px-4 py-8 text-center text-sm text-gray-500">{t(sort === 'relevance' ? 'feed.emptyWeek' : 'feed.empty')}</p>
          ) : sort === 'date' ? (
            days.map(day => (
              <div key={day.key}>
                <h3 className="sticky top-0 z-[1] border-b border-gray-100 bg-gray-50/95 px-4 py-1.5 text-xs font-medium uppercase tracking-wide text-gray-500">{day.label}</h3>
                <div className="divide-y divide-gray-100">{day.items.map(row)}</div>
              </div>
            ))
          ) : (
            <div className="divide-y divide-gray-100">{items.map(row)}</div>
          )}
          {items && items.length < total && (
            <div className="flex justify-center border-t border-gray-100 p-3">
              <Button variant="ghost" size="sm" onClick={() => load(items.length).catch(() => {})}>
                {t('documents.more', { count: total - items.length })}
              </Button>
            </div>
          )}
        </section>
      </div>
    </div>
  );
}
