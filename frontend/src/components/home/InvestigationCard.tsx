'use client';

import { AlertTriangle, Check, CheckCircle2, MoreHorizontal, Plus, ShieldAlert, Siren, Trash2, XCircle, type LucideIcon } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from '@/components/ui/dropdown-menu';
import { ThinkingOrb } from '@/components/ThinkingOrb';
import { formatAgo, formatDate, useI18n, type MessageKey, type Translate } from '@/i18n';
import { formatSize } from '@/services/builtinService';
import type { Highlight, InvestigationStatus } from '@/services/dashboardService';
import type { CollectProgress } from '@/services/investigationService';
import { cn } from '@/lib/utils';

export type Run = CollectProgress['runs'][number];

const WEEKDAYS = [1, 2, 3, 4, 5];

/** The one state an investigation is in now, the most pressing first. */
export function StatusBadge({ status, run }: { status: InvestigationStatus; run?: Run }) {
  const { t, locale } = useI18n();
  const pill = 'inline-flex flex-shrink-0 items-center gap-1.5 rounded-full px-2 py-0.5 text-xs font-medium';
  if (run && run.done < run.total) {
    return (
      <span className={cn(pill, 'bg-blue-50 text-blue-800')}>
        <ThinkingOrb variant="background" size={12} />
        {t('status.collecting', { done: run.done, total: run.total })}
      </span>
    );
  }
  if (status.indexing > 0) {
    return (
      <span className={cn(pill, 'bg-blue-50 text-blue-800')}>
        <ThinkingOrb variant="background" size={12} />
        {t('status.indexing', { count: status.indexing })}
      </span>
    );
  }
  if (status.schedule.schedule.paused) return <span className={cn(pill, 'bg-gray-100 text-gray-700')}>{t('status.paused')}</span>;
  if (!status.schedule.open) {
    const next = status.schedule.nextOpen;
    const at = next && formatDate(locale, next, new Date(next).toDateString() === new Date().toDateString() ? { timeStyle: 'short' } : { weekday: 'short', hour: '2-digit', minute: '2-digit' });
    return <span className={cn(pill, 'bg-amber-50 text-amber-800')}>{at ? t('status.closedUntil', { at }) : t('status.closed')}</span>;
  }
  if (status.failing > 0) return <span className={cn(pill, 'bg-red-50 text-red-700')}>{t('status.failing', { count: status.failing })}</span>;
  return (
    <span className={cn(pill, 'bg-emerald-50 text-emerald-800')}>
      <span className="h-1.5 w-1.5 rounded-full bg-emerald-500" />
      {t('status.upToDate')}
    </span>
  );
}

/** When the investigation may collect: "22:00–06:00 · dias úteis", or any time. */
export function scheduleSummary(t: Translate, locale: string, status: InvestigationStatus): string {
  const { schedule } = status.schedule;
  if (schedule.paused) return t('status.paused');
  const parts: string[] = [];
  if (schedule.window) parts.push(`${schedule.window.start}–${schedule.window.end}`);
  if (schedule.days.length) {
    const weekday = (day: number) => new Intl.DateTimeFormat(locale, { weekday: 'short' }).format(new Date(2026, 0, 4 + day)); // 4 Jan 2026 is a Sunday
    parts.push(schedule.days.join() === WEEKDAYS.join() ? t('status.weekdays') : schedule.days.map(weekday).join(', '));
  }
  return parts.length ? parts.join(' · ') : t('status.anyTime');
}

const HIGHLIGHT: Record<Highlight['kind'], { label: MessageKey; icon: LucideIcon; tone: string }> = {
  contradiction: { label: 'highlight.contradiction', icon: XCircle, tone: 'text-red-700' },
  fulfillment: { label: 'highlight.fulfillment', icon: CheckCircle2, tone: 'text-emerald-700' },
  check_false: { label: 'highlight.checkFalse', icon: XCircle, tone: 'text-red-700' },
  check_true: { label: 'highlight.checkTrue', icon: CheckCircle2, tone: 'text-emerald-700' },
  scandal: { label: 'highlight.scandal', icon: Siren, tone: 'text-red-700' },
};

/** Where a highlight's evidence is: the claim on Promises, the live session it was checked in, or the scandal's news. */
export function highlightHref(highlight: Highlight): string {
  if (highlight.sessionId) return `/live?id=${encodeURIComponent(highlight.sessionId)}`;
  if (highlight.kind === 'scandal' && highlight.documentId) return `/document?id=${encodeURIComponent(highlight.documentId)}`;
  return '/claims';
}

/** The latest finding: a suggestion says so, it is never shown as a fact. */
export function HighlightBlock({ highlight, onOpen }: { highlight: Highlight | null; onOpen: () => void }) {
  const { t, locale } = useI18n();
  if (!highlight) return <p className="rounded-md bg-gray-50 px-3 py-2.5 text-sm text-gray-500">{t('highlight.none')}</p>;
  const { label, icon: Icon, tone } = HIGHLIGHT[highlight.kind];
  const review = highlight.status === 'suggested';
  return (
    <button
      onClick={event => {
        event.stopPropagation();
        onOpen();
      }}
      className="w-full rounded-md bg-gray-50 px-3 py-2.5 text-left hover:bg-gray-100"
    >
      <span className={cn('flex items-center gap-1.5 text-xs font-medium', review ? 'text-amber-800' : tone)}>
        {review ? <ShieldAlert className="h-3.5 w-3.5" /> : <Icon className="h-3.5 w-3.5" />}
        {t(label)}
        {review && <span className="rounded bg-amber-100 px-1.5 py-px text-[11px]">{t('highlight.toReview')}</span>}
      </span>
      <span className="mt-1 line-clamp-2 block text-sm text-gray-900">{highlight.text}</span>
      <span className="mt-1 block truncate text-xs text-gray-500">{[highlight.person, formatAgo(locale, highlight.date)].filter(Boolean).join(' · ')}</span>
    </button>
  );
}

function Figure({ value, label, alert }: { value: number; label: string; alert?: string | null }) {
  const { locale } = useI18n();
  return (
    <div className="min-w-0">
      <div className="text-lg font-semibold tabular-nums text-gray-900">{value.toLocaleString(locale)}</div>
      <div className="truncate text-xs text-gray-500">{label}</div>
      {alert && (
        <div className="flex items-center gap-1 truncate text-xs text-red-700">
          <AlertTriangle className="h-3 w-3 flex-shrink-0" />
          {alert}
        </div>
      )}
    </div>
  );
}

/** The mark of something picked to compare. */
export function PickMark({ picked }: { picked: boolean }) {
  return (
    <span className={cn('flex h-5 w-5 flex-shrink-0 items-center justify-center rounded border', picked ? 'border-gray-900 bg-gray-900 text-white' : 'border-gray-300 bg-white')}>
      {picked && <Check className="h-3.5 w-3.5" strokeWidth={3} />}
    </span>
  );
}

/** What can be done to an investigation without opening it. Its clicks and keys stay out of the
 *  card, which opens on both (the menu is portaled, but React events still bubble to the card). */
function CardMenu({ name, onDelete }: { name: string; onDelete: () => void }) {
  const { t } = useI18n();
  return (
    <div onClick={event => event.stopPropagation()} onKeyDown={event => event.stopPropagation()}>
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button variant="ghost" size="icon" className="-mr-2 -mt-1 h-8 w-8 flex-shrink-0" title={t('common.moreOptions')} aria-label={t('common.moreOptionsOf', { name })}>
            <MoreHorizontal className="h-4 w-4" />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end">
          <DropdownMenuItem onSelect={onDelete} className="text-red-700 focus:text-red-700">
            <Trash2 className="mr-2 h-4 w-4" /> {t('investigation.delete')}
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
    </div>
  );
}

/** A political investigation on Home: its state, what was found last and its numbers. `picked`:
 *  Home is picking investigations to compare (undefined when it is not). */
export function InvestigationCard({
  status,
  run,
  picked,
  onOpen,
  onHighlight,
  onDelete,
}: {
  status: InvestigationStatus;
  run?: Run;
  picked?: boolean;
  onOpen: () => void;
  onHighlight: (highlight: Highlight) => void;
  onDelete?: () => void;
}) {
  const { t, locale } = useI18n();
  const footer = [
    status.lastUpdate ? t('status.updated', { when: formatAgo(locale, status.lastUpdate) }) : t('status.neverUpdated'),
    `≈ ${formatSize(Math.max(1, status.diskBytes / 1048576))}`,
    scheduleSummary(t, locale, status),
  ];
  return (
    <div
      role="button"
      tabIndex={0}
      onClick={onOpen}
      onKeyDown={event => event.key === 'Enter' && onOpen()}
      aria-pressed={picked}
      className={cn(
        'flex cursor-pointer flex-col gap-3 rounded-lg border bg-white p-4 shadow-sm transition hover:shadow focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-blue-500',
        picked ? 'border-gray-900 ring-1 ring-gray-900' : 'border-gray-200 hover:border-gray-300',
      )}
    >
      <div className="flex items-start justify-between gap-3">
        {picked !== undefined && <PickMark picked={picked} />}
        <div className="min-w-0 flex-1">
          <h3 className="truncate font-semibold text-gray-900">{status.name}</h3>
          {status.targets.length > 0 && <p className="truncate text-xs text-gray-500">{status.targets.join(', ')}</p>}
        </div>
        <StatusBadge status={status} run={run} />
        {onDelete && picked === undefined && <CardMenu name={status.name} onDelete={onDelete} />}
      </div>
      <HighlightBlock highlight={status.latest} onOpen={() => (picked !== undefined ? onOpen() : status.latest && onHighlight(status.latest))} />
      <div className="grid grid-cols-4 gap-2">
        <Figure value={status.sources} label={t('home.sources')} alert={status.failing > 0 ? t('status.failingShort', { count: status.failing }) : null} />
        <Figure value={status.documents} label={t('home.documents')} />
        <Figure value={status.insights} label={t('status.insights')} />
        <Figure value={status.contradictions.confirmed} label={t('status.contradictions')} />
      </div>
      <p className="mt-auto truncate text-xs text-gray-500">{footer.join(' · ')}</p>
    </div>
  );
}

/** The dashed card that starts something new. */
export function AddCard({ label, help, onClick, className }: { label: string; help: string; onClick: () => void; className?: string }) {
  return (
    <button
      onClick={onClick}
      className={cn(
        'flex min-h-[10rem] flex-col items-center justify-center gap-2 rounded-lg border-2 border-dashed border-gray-300 p-4 text-center text-gray-600 transition hover:border-gray-400 hover:bg-gray-50 hover:text-gray-900',
        className,
      )}
    >
      <Plus className="h-6 w-6" />
      <span className="font-medium">{label}</span>
      <span className="text-xs text-gray-500">{help}</span>
    </button>
  );
}
