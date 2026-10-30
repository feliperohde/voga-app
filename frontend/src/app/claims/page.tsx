'use client';

import { useCallback, useEffect, useMemo, useState } from 'react';
import { useRouter } from 'next/navigation';
import { AlertTriangle, Check, CheckCircle2, ExternalLink, FileText, RotateCcw, Trash2, X } from 'lucide-react';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import { SelectField } from '@/components/ui/select';
import { useConfirm } from '@/components/ui/confirm-dialog';
import { PageHeader } from '@/components/PageHeader';
import { ThinkingOrb } from '@/components/ThinkingOrb';
import { KIND_ICONS } from '@/components/DocumentList';
import { useInvestigation } from '@/contexts/InvestigationContext';
import { useCollectProgress } from '@/hooks/useCollectProgress';
import { formatDate, useI18n, type MessageKey } from '@/i18n';
import { AnalysisStatus, Claim, claimsService, claimState, ClaimState, Finding, FindingStatus } from '@/services/claimsService';
import { formatClock, investigationService, Target } from '@/services/investigationService';
import { documentHref, officialLinks } from '@/lib/evidence';
import { openExternal } from '@/lib/openExternal';

type View = 'review' | 'contradicted' | 'fulfilled' | 'none' | 'all';

const STATE_STYLE: Record<ClaimState, { label: MessageKey; className: string }> = {
  contradicted: { label: 'claims.state.contradicted', className: 'bg-red-50 text-red-800' },
  fulfilled: { label: 'claims.state.fulfilled', className: 'bg-green-50 text-green-800' },
  review: { label: 'claims.state.review', className: 'bg-amber-50 text-amber-900' },
  none: { label: 'claims.state.none', className: 'bg-gray-100 text-gray-600' },
};

const SCOPE_STYLE: Record<'self' | 'party' | 'ally' | 'other' | 'report', string> = {
  self: 'bg-gray-900 text-white',
  party: 'bg-purple-50 text-purple-800',
  ally: 'bg-blue-50 text-blue-800',
  other: 'bg-gray-100 text-gray-700',
  report: 'bg-gray-100 text-gray-600',
};

function FindingRow({ finding, claimant, onStatus }: { finding: Finding; claimant: string; onStatus: (status: FindingStatus) => void }) {
  const { t, locale } = useI18n();
  const router = useRouter();
  const contradicts = finding.label === 'CONTRADICTS';
  const Icon = contradicts ? AlertTriangle : CheckCircle2;
  const EvidenceIcon = finding.evidence.kind ? KIND_ICONS[finding.evidence.kind] : FileText;
  const href = documentHref(finding.evidence.documentId, finding.startTime);
  const title =
    finding.status === 'suggested'
      ? t(contradicts ? 'claims.finding.possibleContradiction' : 'claims.finding.possibleFulfillment')
      : finding.status === 'confirmed'
        ? t(contradicts ? 'claims.finding.contradiction' : 'claims.finding.fulfillment')
        : t('claims.finding.rejected');
  const scope = finding.actor?.scope ?? 'report';
  const own = scope === 'self' || scope === 'report';
  const tone = finding.status === 'rejected' || !own ? 'border-gray-200' : contradicts ? 'border-red-200' : 'border-green-200';
  const actorName = finding.actor ? `${finding.actor.name}${finding.actor.party ? ` (${finding.actor.party})` : ''}` : '';
  const actorLine =
    scope === 'self' || scope === 'other' || scope === 'report'
      ? t(`claims.actor.${scope}`, { name: claimant, actor: actorName })
      : t(`claims.actor.${scope}.${finding.label}`, { name: claimant, actor: actorName });
  const links = officialLinks(finding.evidence.kind, finding.evidence.url);

  return (
    <div className={`rounded-md border bg-white p-3 ${tone} ${finding.status === 'rejected' ? 'opacity-60' : ''}`}>
      <div className="flex items-start gap-2">
        <Icon className={`mt-0.5 h-4 w-4 flex-shrink-0 ${contradicts ? 'text-red-600' : 'text-green-600'}`} />
        <div className="min-w-0 flex-1 space-y-1.5">
          <div className="flex flex-wrap items-center gap-x-2 text-sm">
            <span className="font-medium text-gray-900">{title}</span>
            <span className={`rounded px-1.5 py-0.5 text-xs font-medium ${SCOPE_STYLE[scope]}`}>{t(`claims.scope.${scope}`)}</span>
            {finding.status === 'suggested' && (
              <span className="text-xs text-gray-500">{t('claims.finding.confidence', { value: Math.round(finding.confidence * 100) })}</span>
            )}
          </div>
          <button
            disabled={!href}
            onClick={() => href && router.push(href)}
            className="flex max-w-full items-center gap-1.5 text-left text-sm text-blue-700 hover:underline disabled:text-gray-700 disabled:no-underline"
          >
            <EvidenceIcon className="h-3.5 w-3.5 flex-shrink-0" />
            <span className="truncate">{finding.evidence.title}</span>
            <span className="flex-shrink-0 text-xs text-gray-500">
              · {formatDate(locale, finding.date, { dateStyle: 'medium' })}
              {finding.startTime != null && ` · ${formatClock(finding.startTime)}`}
            </span>
          </button>
          <p className={`text-xs ${own ? 'text-gray-600' : 'font-medium text-gray-800'}`}>{actorLine}</p>
          {(links.page || links.allVotes) && (
            <div className="flex flex-wrap gap-x-3 text-xs">
              {links.page && (
                <button onClick={() => openExternal(links.page!)} className="inline-flex items-center gap-1 text-blue-700 hover:underline">
                  <ExternalLink className="h-3 w-3" /> {t('claims.officialSource')}
                </button>
              )}
              {links.allVotes && (
                <button onClick={() => openExternal(links.allVotes!)} className="inline-flex items-center gap-1 text-blue-700 hover:underline">
                  <ExternalLink className="h-3 w-3" /> {t('claims.allVotes')}
                </button>
              )}
            </div>
          )}
          {finding.excerpt && <p className="line-clamp-3 select-text text-sm text-gray-700">{finding.excerpt}</p>}
          {finding.rationale && (
            <p className="text-xs text-gray-600">
              <span className="font-medium">{t('claims.finding.why')}</span> {finding.rationale}
            </p>
          )}
        </div>
        <div className="flex flex-shrink-0 gap-1">
          {finding.status === 'suggested' ? (
            <>
              <Button variant="outline" size="sm" className="h-7 px-2 text-xs" onClick={() => onStatus('confirmed')}>
                <Check className="mr-1 h-3.5 w-3.5" /> {t('claims.confirm')}
              </Button>
              <Button variant="ghost" size="sm" className="h-7 px-2 text-xs" onClick={() => onStatus('rejected')}>
                <X className="mr-1 h-3.5 w-3.5" /> {t('claims.reject')}
              </Button>
            </>
          ) : (
            <Button variant="ghost" size="sm" className="h-7 px-2 text-xs" title={t('claims.undo')} onClick={() => onStatus('suggested')}>
              <RotateCcw className="h-3.5 w-3.5" />
            </Button>
          )}
        </div>
      </div>
    </div>
  );
}

function ClaimCard({ claim, person, onChanged }: { claim: Claim; person: string; onChanged: () => void }) {
  const { t, locale } = useI18n();
  const router = useRouter();
  const confirm = useConfirm();
  const state = claimState(claim);
  const style = STATE_STYLE[state];
  const SourceIcon = claim.source.kind ? KIND_ICONS[claim.source.kind] : FileText;
  const href = documentHref(claim.source.documentId, claim.startTime);
  const findings = [...claim.findings].sort((a, b) => Number(a.status === 'rejected') - Number(b.status === 'rejected'));

  const setStatus = (finding: Finding, status: FindingStatus) =>
    claimsService
      .setStatus(finding.id, status)
      .then(onChanged)
      .catch(error => toast.error(String(error)));
  const remove = async () => {
    if (await confirm({ title: t('claims.deleteTitle'), description: t('claims.deleteHelp'), destructive: true, confirmLabel: t('common.delete') })) {
      claimsService.remove(claim.id).then(onChanged).catch(error => toast.error(String(error)));
    }
  };

  return (
    <article className="space-y-3 rounded-lg border border-gray-200 bg-white p-5 shadow-sm">
      <div className="flex flex-wrap items-center gap-2 text-xs">
        <span className="rounded bg-gray-900 px-1.5 py-0.5 font-medium text-white">{t(claim.kind === 'promise' ? 'claims.kind.promise' : 'claims.kind.statement')}</span>
        {(claim.kind === 'promise' || state !== 'none') && <span className={`rounded px-1.5 py-0.5 font-medium ${style.className}`}>{t(style.label)}</span>}
        {claim.topic && <span className="rounded bg-gray-100 px-1.5 py-0.5 text-gray-700">{claim.topic}</span>}
        <span className="text-gray-500">
          {t('claims.saidBy', { name: person })} · {formatDate(locale, claim.statedAt, { dateStyle: 'medium' })}
        </span>
        <button onClick={remove} title={t('claims.delete')} className="ml-auto text-gray-400 hover:text-gray-700">
          <Trash2 className="h-4 w-4" />
        </button>
      </div>
      <h3 className="text-base font-semibold text-gray-900">{claim.text}</h3>
      <blockquote className="select-text border-l-2 border-gray-300 pl-3 text-sm italic text-gray-700">“{claim.quote}”</blockquote>
      <button
        disabled={!href}
        onClick={() => href && router.push(href)}
        className="flex max-w-full items-center gap-1.5 text-xs text-gray-500 hover:text-gray-800 disabled:hover:text-gray-500"
      >
        <SourceIcon className="h-3.5 w-3.5 flex-shrink-0" />
        <span className="truncate">
          {[claim.source.title, claim.source.site].filter(Boolean).join(' · ')}
          {claim.startTime != null && ` · ${formatClock(claim.startTime)}`}
        </span>
      </button>
      {findings.length > 0 && (
        <div className="space-y-2 border-t border-gray-100 pt-3">
          {findings.map(finding => (
            <FindingRow key={finding.id} finding={finding} claimant={person} onStatus={status => setStatus(finding, status)} />
          ))}
        </div>
      )}
    </article>
  );
}

function matches(claim: Claim, view: View) {
  const state = claimState(claim);
  switch (view) {
    case 'review':
      return state === 'review';
    case 'none':
      return claim.kind === 'promise' && state === 'none';
    case 'all':
      return true;
    default:
      return state === view;
  }
}

export default function ClaimsPage() {
  const { t } = useI18n();
  const { current } = useInvestigation();
  const progress = useCollectProgress();
  const [claims, setClaims] = useState<Claim[] | null>(null);
  const [targets, setTargets] = useState<Target[]>([]);
  const [status, setStatus] = useState<AnalysisStatus | null>(null);
  const [view, setView] = useState<View>('all');
  const [person, setPerson] = useState('all');
  const [kind, setKind] = useState<'all' | 'promise' | 'statement'>('all');

  const load = useCallback(async () => {
    if (!current) return;
    const [claims, targets, status] = await Promise.all([
      claimsService.list(current.id),
      investigationService.targets(current.id),
      claimsService.status(current.id),
    ]);
    setClaims(claims);
    setTargets(targets);
    setStatus(status);
  }, [current]);

  useEffect(() => {
    setClaims(null);
    load().catch(error => console.error('Failed to load claims:', error));
  }, [load]);
  // New claims arrive while the AI reads
  useEffect(() => {
    if (progress && !progress.analyzing) load().catch(() => {});
  }, [progress?.analyzing, progress, load]);

  const names = useMemo(() => new Map(targets.map(target => [target.id, target.name])), [targets]);
  const counts = useMemo(() => {
    const all = claims ?? [];
    return {
      review: all.filter(c => claimState(c) === 'review').length,
      contradicted: all.filter(c => claimState(c) === 'contradicted').length,
      fulfilled: all.filter(c => claimState(c) === 'fulfilled').length,
      none: all.filter(c => c.kind === 'promise' && claimState(c) === 'none').length,
      all: all.length,
    };
  }, [claims]);

  if (!current) return <PageHeader title={t('nav.claims')} />;
  const shown = (claims ?? []).filter(c => matches(c, view) && (person === 'all' || c.targetId === person) && (kind === 'all' || c.kind === kind));
  const views: View[] = ['all', 'review', 'contradicted', 'fulfilled', 'none'];

  return (
    <div>
      <PageHeader title={t('nav.claims')} />
      <div className="mx-auto max-w-4xl space-y-4 px-4 py-6 sm:px-8">
        <p className="rounded-md border border-blue-100 bg-blue-50 p-3 text-sm text-blue-900">{t('claims.notice')}</p>

        {(progress?.analyzing || (status?.pending ?? 0) > 0) && (
          <div className="flex items-center gap-2 text-sm text-gray-600">
            {progress?.analyzing && <ThinkingOrb variant="thinking" size={18} />}
            {progress?.analyzing && progress.current
              ? t('claims.reading', { title: progress.current })
              : t('claims.pending', { count: status?.pending ?? 0 })}
          </div>
        )}

        <div className="flex flex-wrap items-center gap-2">
          {views.map(v => (
            <Button key={v} variant={view === v ? 'default' : 'outline'} size="sm" onClick={() => setView(v)}>
              {t(`claims.view.${v}`)} <span className="ml-1.5 opacity-70">{counts[v]}</span>
            </Button>
          ))}
          <div className="ml-auto flex gap-2">
            <div className="w-44">
              <SelectField
                value={person}
                onValueChange={setPerson}
                aria-label={t('claims.person')}
                options={[{ value: 'all', label: t('claims.everyone') }, ...targets.filter(t => t.kind === 'person').map(t => ({ value: t.id, label: t.name }))]}
              />
            </div>
            <div className="w-56">
              <SelectField
                value={kind}
                onValueChange={value => setKind(value as typeof kind)}
                aria-label={t('claims.kindFilter')}
                options={[
                  { value: 'all', label: t('claims.kind.all') },
                  { value: 'promise', label: t('claims.kind.promises') },
                  { value: 'statement', label: t('claims.kind.statements') },
                ]}
              />
            </div>
          </div>
        </div>

        {claims === null ? (
          <div className="flex justify-center p-8">
            <ThinkingOrb variant="working" size={24} />
          </div>
        ) : shown.length === 0 ? (
          <p className="py-10 text-center text-sm text-gray-500">{claims.length === 0 ? t('claims.empty') : t('claims.emptyView')}</p>
        ) : (
          shown.map(claim => <ClaimCard key={claim.id} claim={claim} person={names.get(claim.targetId) ?? ''} onChanged={() => load().catch(() => {})} />)
        )}
      </div>
    </div>
  );
}
