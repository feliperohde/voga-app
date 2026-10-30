'use client';

import { useRouter } from 'next/navigation';
import { formatDate, useI18n } from '@/i18n';
import { formatClock } from '@/services/investigationService';
import { LiveSessionSummary, sessionHref } from '@/services/liveService';

/** A past session (or debate) in a list: who, when, how long, and its verdicts. */
export function SessionRow({ session }: { session: LiveSessionSummary }) {
  const { t, locale } = useI18n();
  const router = useRouter();
  const duration = session.endedAt ? (new Date(session.endedAt).getTime() - new Date(session.startedAt).getTime()) / 1000 : null;
  const chips: [number, string, string][] = [
    [session.falseCount, 'bg-red-50 text-red-800', t('live.count.false', { count: session.falseCount })],
    [session.partlyCount, 'bg-amber-50 text-amber-900', t('live.count.partly', { count: session.partlyCount })],
    [session.trueCount, 'bg-green-50 text-green-800', t('live.count.true', { count: session.trueCount })],
  ];
  return (
    <button onClick={() => router.push(sessionHref(session.id, !session.investigationId))} className="flex w-full flex-wrap items-center gap-x-3 gap-y-1 rounded-md px-3 py-2.5 text-left hover:bg-gray-50">
      <span className="min-w-0 flex-1">
        <span className="block truncate text-sm font-medium text-gray-900">{session.title}</span>
        <span className="block text-xs text-gray-500">
          {[session.targetName, formatDate(locale, session.startedAt, { dateStyle: 'medium', timeStyle: 'short' }), duration != null && formatClock(duration)].filter(Boolean).join(' · ')}
          {session.status === 'interrupted' && ` · ${t('live.interrupted')}`}
          {session.status === 'recording' && ` · ${t('live.recordingNow')}`}
        </span>
      </span>
      {chips
        .filter(([count]) => count > 0)
        .map(([, style, label]) => (
          <span key={label} className={`rounded px-1.5 py-0.5 text-xs font-medium ${style}`}>
            {label}
          </span>
        ))}
    </button>
  );
}
