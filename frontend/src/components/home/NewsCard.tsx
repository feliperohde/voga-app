'use client';

import { useState } from 'react';
import { KIND_ICONS } from '@/components/DocumentList';
import { formatAgo, useI18n } from '@/i18n';
import { imageSource, type FeedItem } from '@/services/feedService';
import { cn, plainText } from '@/lib/utils';

/**
 * A news item as a card with its picture (or, without one, its kind and site). `big`: the first
 * of the Journal, with its opening lines. `actions` sit under the text (star, more, less).
 */
export function NewsCard({ item, big, onOpen, actions }: { item: FeedItem; big?: boolean; onOpen: () => void; actions?: React.ReactNode }) {
  const { t, locale } = useI18n();
  const [broken, setBroken] = useState(false);
  const src = broken ? null : imageSource(item);
  const Icon = KIND_ICONS[item.kind];
  const meta = [item.site, item.investigationName, formatAgo(locale, item.publishedAt ?? item.fetchedAt)].filter(Boolean);
  return (
    <article
      className={cn(
        'flex flex-col overflow-hidden rounded-lg border border-gray-200 bg-white shadow-sm transition hover:border-gray-300 hover:shadow',
        big && 'lg:col-span-2 lg:row-span-2',
        item.feedback === -1 && 'opacity-60',
      )}
    >
      <button onClick={onOpen} className="flex flex-1 flex-col text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-blue-500">
        <div className="relative aspect-[16/9] w-full overflow-hidden bg-gray-100">
          {src ? (
            // eslint-disable-next-line @next/next/no-img-element -- static export: no image optimization
            <img src={src} alt="" loading="lazy" referrerPolicy="no-referrer" onError={() => setBroken(true)} className="h-full w-full object-cover" />
          ) : (
            <div className="flex h-full w-full flex-col items-center justify-center gap-2 text-gray-400">
              <Icon className={big ? 'h-10 w-10' : 'h-7 w-7'} />
              {item.site && <span className="max-w-[80%] truncate text-xs font-medium uppercase tracking-wide">{item.site}</span>}
            </div>
          )}
        </div>
        <div className="flex flex-1 flex-col gap-1.5 p-3">
          <span className={cn('flex items-start gap-2 text-sm', big && 'lg:text-lg')}>
            {!item.read && <span className="mt-1.5 h-2 w-2 flex-shrink-0 rounded-full bg-blue-500" aria-label={t('feed.unread')} />}
            <span className={cn('line-clamp-3', item.read ? 'font-medium text-gray-700' : 'font-semibold text-gray-900')}>{item.title}</span>
          </span>
          {big && <span className="line-clamp-3 hidden text-sm text-gray-600 lg:block">{plainText(item.excerpt)}</span>}
          <span className="mt-auto truncate pt-1 text-xs text-gray-500">{meta.join(' · ')}</span>
        </div>
      </button>
      {actions && <div className="flex items-center justify-end gap-0.5 border-t border-gray-100 px-1.5 py-1">{actions}</div>}
    </article>
  );
}
