'use client';

import { useRouter } from 'next/navigation';
import { AlertTriangle, Check, CheckCircle2, CircleHelp, ExternalLink, FileText, Globe, Play, RotateCcw, Scale, Search, X } from 'lucide-react';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import { ThinkingOrb } from '@/components/ThinkingOrb';
import { KIND_ICONS } from '@/components/DocumentList';
import { formatDate, useI18n, type MessageKey, type Translate } from '@/i18n';
import { documentHref, officialLinks } from '@/lib/evidence';
import { openExternal } from '@/lib/openExternal';
import { formatClock } from '@/services/investigationService';
import { CheckStatus, LiveCheck, liveService, Verdict } from '@/services/liveService';

/** Each verdict: its color, icon, and how it reads while a suggestion and once confirmed. */
export const VERDICT_STYLE: Record<Exclude<Verdict, 'checking' | 'detected'>, { tone: string; border: string; icon: typeof Check; suggested: MessageKey; confirmed: MessageKey }> = {
  true: { tone: 'bg-green-50 text-green-800', border: 'border-green-200', icon: CheckCircle2, suggested: 'live.verdict.true', confirmed: 'live.verdict.true.confirmed' },
  false: { tone: 'bg-red-50 text-red-800', border: 'border-red-200', icon: AlertTriangle, suggested: 'live.verdict.false', confirmed: 'live.verdict.false.confirmed' },
  partly: { tone: 'bg-amber-50 text-amber-900', border: 'border-amber-200', icon: Scale, suggested: 'live.verdict.partly', confirmed: 'live.verdict.partly.confirmed' },
  unverified: { tone: 'bg-gray-100 text-gray-700', border: 'border-gray-200', icon: CircleHelp, suggested: 'live.verdict.unverified', confirmed: 'live.verdict.unverified' },
};

/** Asks for a check now: shown as being checked until the verdict comes as an event. */
export function startCheck(check: LiveCheck, onChanged: (check: LiveCheck) => void, t: Translate) {
  onChanged({ ...check, verdict: 'checking' });
  liveService.checkNow(check.id).catch(error => {
    onChanged(check);
    toast.error(t('live.checkFailed'), { description: String(error) });
  });
}

interface FactCheckCardProps {
  check: LiveCheck;
  /** Who said it, when the session follows more than one person. */
  person: string | null;
  hasAudio: boolean;
  onChanged: (check: LiveCheck) => void;
}

/** A claim heard live and what the records say about it: a suggestion until a person confirms it. */
export function FactCheckCard({ check, person, hasAudio, onChanged }: FactCheckCardProps) {
  const { t, locale } = useI18n();
  const router = useRouter();
  const checking = check.verdict === 'checking';
  const detected = check.verdict === 'detected';
  const style = checking || detected ? null : VERDICT_STYLE[check.verdict as Exclude<Verdict, 'checking' | 'detected'>];
  const rejected = check.status === 'rejected';
  const Icon = style?.icon ?? CircleHelp;

  const setStatus = (status: CheckStatus) =>
    liveService
      .setCheckStatus(check.id, status)
      .then(() => onChanged({ ...check, status }))
      .catch(error => toast.error(t('common.saveFailed'), { description: String(error) }));
  const checkNow = () => startCheck(check, onChanged, t);
  const listen = () => liveService.playClip(check.sessionId, check.start, check.end).catch(error => toast.error(t('live.playFailed'), { description: String(error) }));

  const label = checking
    ? t('live.verdict.checking')
    : detected
      ? t('live.verdict.detected')
    : rejected
      ? t('live.verdict.rejected')
      : t(check.status === 'confirmed' ? style!.confirmed : style!.suggested);

  return (
    <article id={`check-${check.id}`} className={`space-y-2 rounded-lg border bg-white p-4 shadow-sm ${rejected ? 'border-gray-200 opacity-60' : (style?.border ?? 'border-gray-200')}`}>
      <div className="flex flex-wrap items-center gap-2 text-xs">
        {checking ? (
          <span className="flex items-center gap-1.5 rounded bg-gray-100 px-1.5 py-0.5 font-medium text-gray-700">
            <ThinkingOrb variant="thinking" size={14} /> {label}
          </span>
        ) : detected ? (
          <Button variant="outline" size="sm" className="h-6 px-2 text-xs" onClick={checkNow}>
            <Search className="h-3 w-3" /> {t('live.checkNow')}
          </Button>
        ) : (
          <span className={`flex items-center gap-1 rounded px-1.5 py-0.5 font-medium ${rejected ? 'bg-gray-100 text-gray-600' : style!.tone}`}>
            <Icon className="h-3.5 w-3.5" /> {label}
          </span>
        )}
        <span className="rounded bg-gray-100 px-1.5 py-0.5 text-gray-600">{t(`live.kind.${check.kind}`)}</span>
        {!checking && check.verdict !== 'unverified' && check.status === 'suggested' && check.confidence != null && (
          <span className="text-gray-500">{t('claims.finding.confidence', { value: Math.round(check.confidence * 100) })}</span>
        )}
        {check.webSearched && (
          <span className="flex items-center gap-1 text-gray-500" title={t('live.check.webHelp')}>
            <Globe className="h-3 w-3" /> {t('live.check.web')}
          </span>
        )}
        <button
          disabled={!hasAudio}
          onClick={listen}
          title={hasAudio ? t('live.listen') : undefined}
          className="ml-auto flex items-center gap-1 font-mono text-gray-500 hover:text-gray-900 disabled:hover:text-gray-500"
        >
          {hasAudio && <Play className="h-3 w-3" />}
          {formatClock(check.start)}
        </button>
      </div>

      <h3 className="text-sm font-semibold text-gray-900">{check.claim}</h3>
      <p className="select-text text-sm italic text-gray-600">
        {person ? `${person}: ` : ''}“{check.quote}”
      </p>

      {check.evidence.map(evidence => {
        const EvidenceIcon = KIND_ICONS[evidence.kind] ?? FileText;
        const href = documentHref(evidence.documentId, evidence.startTime);
        const links = officialLinks(evidence.kind, evidence.url);
        const external = links.page ?? (evidence.url.startsWith('http') ? evidence.url : null);
        return (
          <div key={evidence.chunkId} className="space-y-1 rounded-md bg-gray-50 p-2.5">
            <button onClick={() => href && router.push(href)} className="flex max-w-full items-center gap-1.5 text-left text-sm text-blue-700 hover:underline">
              <EvidenceIcon className="h-3.5 w-3.5 flex-shrink-0" />
              <span className="truncate">{evidence.title}</span>
              <span className="flex-shrink-0 text-xs text-gray-500">· {formatDate(locale, evidence.date, { dateStyle: 'medium' })}</span>
            </button>
            {evidence.actor && evidence.actor.scope !== 'self' && (
              <p className="text-xs font-medium text-gray-800">{t('live.check.actOf', { name: evidence.actor.name + (evidence.actor.party ? ` (${evidence.actor.party})` : '') })}</p>
            )}
            <p className="line-clamp-3 select-text text-xs text-gray-700">{evidence.excerpt}</p>
            {external && (
              <button onClick={() => openExternal(external)} className="inline-flex items-center gap-1 text-xs text-blue-700 hover:underline">
                <ExternalLink className="h-3 w-3" /> {t(links.page ? 'claims.officialSource' : 'document.openOriginal')}
              </button>
            )}
          </div>
        );
      })}

      {check.rationale && (
        <p className="text-xs text-gray-600">
          <span className="font-medium">{t('claims.finding.why')}</span> {check.rationale}
        </p>
      )}
      {check.verdict === 'unverified' && !check.rationale && (
        <p className="text-xs text-gray-500">{t(check.error === 'interrupted' ? 'live.check.interrupted' : 'live.check.noRecord')}</p>
      )}
      {check.verdict === 'unverified' && (
        <Button variant="ghost" size="sm" className="h-7 px-2 text-xs" onClick={checkNow}>
          <RotateCcw className="mr-1 h-3.5 w-3.5" /> {t('live.checkAgain')}
        </Button>
      )}
      {detected && check.error && <p className="text-xs text-red-600">{check.error}</p>}

      {style && check.verdict !== 'unverified' && (
        <div className="flex gap-1">
          {check.status === 'suggested' ? (
            <>
              <Button variant="outline" size="sm" className="h-7 px-2 text-xs" onClick={() => setStatus('confirmed')}>
                <Check className="mr-1 h-3.5 w-3.5" /> {t('claims.confirm')}
              </Button>
              <Button variant="ghost" size="sm" className="h-7 px-2 text-xs" onClick={() => setStatus('rejected')}>
                <X className="mr-1 h-3.5 w-3.5" /> {t('claims.reject')}
              </Button>
            </>
          ) : (
            <Button variant="ghost" size="sm" className="h-7 px-2 text-xs" title={t('claims.undo')} onClick={() => setStatus('suggested')}>
              <RotateCcw className="h-3.5 w-3.5" />
            </Button>
          )}
        </div>
      )}
    </article>
  );
}
