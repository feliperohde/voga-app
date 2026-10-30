'use client';

import { useEffect, useState } from 'react';
import { useRouter } from 'next/navigation';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { Sparkles } from 'lucide-react';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import { PageHeader } from '@/components/PageHeader';
import { ThinkingOrb } from '@/components/ThinkingOrb';
import { CitationCard } from '@/components/CitationCard';
import { useInvestigation } from '@/contexts/InvestigationContext';
import { formatDate, useI18n, type MessageKey } from '@/i18n';
import { KIND_ICON } from '@/lib/nav';
import { compareService, type Comparison, type CompareSummary, type Passage } from '@/services/compareService';

const KINDS: Record<string, MessageKey> = {
  party: 'compare.kind.party',
  organization: 'compare.kind.organization',
  candidacy: 'compare.kind.candidacy',
  bill: 'compare.kind.bill',
  amendment: 'compare.kind.amendment',
  person: 'compare.kind.person',
};

const TIES: Record<string, MessageKey> = {
  MEMBER_OF: 'compare.tie.MEMBER_OF',
  AUTHORED: 'compare.tie.AUTHORED',
  SPENT_WITH: 'compare.tie.SPENT_WITH',
  CANDIDATE_IN: 'compare.tie.CANDIDATE_IN',
  PAID_TO: 'compare.tie.PAID_TO',
  FUNDS: 'compare.tie.FUNDS',
  CONTRACTED: 'compare.tie.CONTRACTED',
  HEADS: 'compare.tie.HEADS',
  PARTNER_OF: 'compare.tie.PARTNER_OF',
  ABOUT: 'compare.tie.ABOUT',
};

function Section({ title, help, children }: { title: string; help?: string; children: React.ReactNode }) {
  return (
    <section className="rounded-lg border border-gray-200 bg-white shadow-sm">
      <div className="border-b border-gray-100 px-4 py-3">
        <h2 className="font-semibold text-gray-900">{title}</h2>
        {help && <p className="text-xs text-gray-500">{help}</p>}
      </div>
      {children}
    </section>
  );
}

function Empty({ text }: { text: string }) {
  return <p className="px-4 py-4 text-sm text-gray-500">{text}</p>;
}

/** Documents per week of each side, one row of bars each, on the same scale. */
function Timeline({ comparison }: { comparison: Comparison }) {
  const { t, locale } = useI18n();
  const top = Math.max(1, ...comparison.timeline.flatMap(w => w.counts));
  return (
    <div className="space-y-3 px-4 py-4">
      {comparison.sides.map((side, i) => (
        <div key={side.id} className="grid grid-cols-[minmax(0,10rem)_minmax(0,1fr)] items-end gap-3">
          <span className="truncate text-sm text-gray-700">{side.name}</span>
          <div className="flex h-10 items-end gap-0.5" role="img" aria-label={t('compare.timelineOf', { name: side.name })}>
            {comparison.timeline.map(week => (
              <div key={week.week} className="group relative flex h-full flex-1 items-end">
                <div className="w-full rounded-t-sm bg-gray-700" style={{ height: `${(week.counts[i] / top) * 100}%`, minHeight: week.counts[i] ? 2 : 0 }} />
                <span className="pointer-events-none absolute bottom-full left-1/2 z-10 mb-1 hidden -translate-x-1/2 whitespace-nowrap rounded bg-gray-900 px-1.5 py-0.5 text-[11px] text-white group-hover:block">
                  {t('compare.week', { date: formatDate(locale, week.week, { day: '2-digit', month: 'short' }), count: week.counts[i] })}
                </span>
              </div>
            ))}
          </div>
        </div>
      ))}
      <div className="grid grid-cols-[minmax(0,10rem)_minmax(0,1fr)] gap-3 text-[11px] text-gray-500">
        <span />
        <span className="flex justify-between">
          <span>{comparison.timeline[0] && formatDate(locale, comparison.timeline[0].week, { day: '2-digit', month: 'short' })}</span>
          <span>{t('compare.thisWeek')}</span>
        </span>
      </div>
    </div>
  );
}

function PassageRow({ passage, sideName }: { passage: Passage; sideName: string }) {
  const { locale } = useI18n();
  const router = useRouter();
  return (
    <li>
      <button onClick={() => router.push(`/document?id=${encodeURIComponent(passage.documentId)}`)} className="w-full px-4 py-2.5 text-left hover:bg-gray-50">
        <span className="block truncate text-sm font-medium text-gray-900">{passage.title}</span>
        <span className="block truncate text-xs text-gray-500">{[sideName, passage.site, formatDate(locale, passage.date)].filter(Boolean).join(' · ')}</span>
      </button>
    </li>
  );
}

/** Several investigations side by side: where they connect, where they diverge, and the AI's write-up. */
export default function ComparePage() {
  const { t } = useI18n();
  const router = useRouter();
  const { scope } = useInvestigation();
  const ids = scope.map(i => i.id).join(',');
  const [comparison, setComparison] = useState<Comparison | null>(null);
  const [summary, setSummary] = useState<CompareSummary | null>(null);
  const [writing, setWriting] = useState(false);

  useEffect(() => {
    if (!ids.includes(',')) return;
    setComparison(null);
    setSummary(null);
    compareService
      .compare(ids.split(','))
      .then(setComparison)
      .catch(error => toast.error(t('compare.failed'), { description: String(error) }));
  }, [ids, t]);

  const write = async () => {
    setWriting(true);
    try {
      setSummary(await compareService.summary(ids.split(',')));
    } catch (error) {
      toast.error(t('compare.summaryFailed'), { description: String(error) });
    } finally {
      setWriting(false);
    }
  };

  if (scope.length < 2) {
    return (
      <div>
        <PageHeader title={t('nav.compare')} />
        <Empty text={t('compare.pickTwo')} />
      </div>
    );
  }
  const name = (id: string) => scope.find(i => i.id === id)?.name ?? '';
  const politics = scope.filter(i => i.kind === 'politics').length;

  return (
    <div>
      <PageHeader title={scope.map(i => i.name).join(' × ')} />
      <div className="mx-auto max-w-5xl space-y-6 px-4 py-6 sm:px-8">
        {!comparison ? (
          <div className="flex justify-center py-12">
            <ThinkingOrb variant="background" size={28} />
          </div>
        ) : (
          <>
            <div className="grid gap-3" style={{ gridTemplateColumns: `repeat(${Math.min(comparison.sides.length, 4)}, minmax(0, 1fr))` }}>
              {comparison.sides.map(side => {
                const Icon = KIND_ICON[side.kind];
                return (
                  <div key={side.id} className="rounded-lg border border-gray-200 bg-white p-4 shadow-sm">
                    <div className="flex items-center gap-2 font-semibold text-gray-900">
                      <Icon className="h-4 w-4 flex-shrink-0 text-gray-500" />
                      <span className="truncate">{side.name}</span>
                    </div>
                    {side.targets.length > 0 && <p className="truncate text-xs text-gray-500">{side.targets.join(', ')}</p>}
                    <p className="mt-2 text-sm text-gray-600">{t('compare.documents', { count: side.documents })}</p>
                  </div>
                );
              })}
            </div>

            <Section title={t('compare.summary')} help={t('compare.summaryHelp')}>
              <div className="space-y-4 px-4 py-4">
                {summary ? (
                  <>
                    <div className="prose prose-sm max-w-none select-text dark:prose-invert">
                      <ReactMarkdown remarkPlugins={[remarkGfm]}>{summary.text}</ReactMarkdown>
                    </div>
                    <div className="grid gap-2 sm:grid-cols-2">
                      {summary.citations.map(citation => (
                        <CitationCard key={citation.index} citation={citation} />
                      ))}
                    </div>
                    <p className="text-xs text-gray-500">{t('compare.summaryNote')}</p>
                  </>
                ) : writing ? (
                  <div className="flex items-center gap-3 text-sm text-gray-600">
                    <ThinkingOrb variant="thinking" size={22} />
                    {t('compare.writing')}
                  </div>
                ) : (
                  <Button onClick={write}>
                    <Sparkles className="mr-2 h-4 w-4" /> {t('compare.write')}
                  </Button>
                )}
              </div>
            </Section>

            {comparison.notice && <p className="rounded-md border border-amber-200 bg-amber-50 p-3 text-sm text-amber-900">{t('compare.noVectors')}</p>}

            <Section title={t('compare.connect')} help={t('compare.connectHelp')}>
              {comparison.bridges.length === 0 ? (
                <Empty text={t('compare.noBridges')} />
              ) : (
                <ul className="divide-y divide-gray-100">
                  {comparison.bridges.map(passage => (
                    <PassageRow key={passage.documentId} passage={passage} sideName={name(passage.investigationId)} />
                  ))}
                </ul>
              )}
              {comparison.sameDocuments.length > 0 && (
                <div className="border-t border-gray-100 px-4 py-3">
                  <h3 className="text-sm font-medium text-gray-900">{t('compare.sameDocuments', { count: comparison.sameDocuments.length })}</h3>
                  <ul className="mt-1 space-y-0.5">
                    {comparison.sameDocuments.slice(0, 8).map(document => (
                      <li key={document.url}>
                        <button onClick={() => router.push(`/document?id=${encodeURIComponent(document.documentId)}`)} className="truncate text-left text-sm text-blue-700 hover:underline">
                          {document.title}
                        </button>
                      </li>
                    ))}
                  </ul>
                </div>
              )}
            </Section>

            {politics > 0 && (
              <Section title={t('compare.shared')} help={t('compare.sharedHelp')}>
                {comparison.shared.length === 0 ? (
                  <Empty text={t('compare.noShared')} />
                ) : (
                  <ul className="divide-y divide-gray-100">
                    {comparison.shared.map(entity => (
                      <li key={entity.key} className="px-4 py-2.5">
                        <div className="text-sm font-medium text-gray-900">
                          {entity.label} <span className="text-xs font-normal text-gray-500">· {KINDS[entity.kind] ? t(KINDS[entity.kind]) : entity.kind}</span>
                        </div>
                        <ul className="mt-0.5 space-y-0.5">
                          {entity.ties.slice(0, 6).map((tie, i) => (
                            <li key={i} className="text-xs text-gray-600">
                              {name(tie.investigationId)}: {tie.who} · {TIES[tie.label] ? t(TIES[tie.label]) : tie.label}
                            </li>
                          ))}
                        </ul>
                      </li>
                    ))}
                  </ul>
                )}
              </Section>
            )}

            {politics > 1 && (
              <Section title={t('compare.votes')} help={t('compare.votesHelp')}>
                {comparison.votes.both === 0 ? (
                  <Empty text={t('compare.noVotes')} />
                ) : (
                  <div className="px-4 py-3">
                    <p className="text-sm text-gray-800">
                      {t('compare.agreement', { agreed: comparison.votes.agreed, both: comparison.votes.both, percent: Math.round((comparison.votes.agreed / comparison.votes.both) * 100) })}
                    </p>
                    {comparison.votes.disagreements.length > 0 && (
                      <ul className="mt-2 divide-y divide-gray-100 rounded-md border border-gray-100">
                        {comparison.votes.disagreements.map(diff => (
                          <li key={diff.label} className="px-3 py-2">
                            <div className="text-sm text-gray-900">{diff.label}</div>
                            <div className="text-xs text-gray-600">{diff.votes.map(v => `${v.who}: ${v.vote}`).join(' · ')}</div>
                          </li>
                        ))}
                      </ul>
                    )}
                  </div>
                )}
              </Section>
            )}

            {politics > 0 && comparison.topics.length > 0 && (
              <Section title={t('compare.topics')} help={t('compare.topicsHelp')}>
                <div className="overflow-x-auto">
                  <table className="w-full text-sm">
                    <thead>
                      <tr className="border-b border-gray-100 text-left text-xs text-gray-500">
                        {comparison.sides.map(side => (
                          <th key={side.id} className="px-4 py-2 font-medium">
                            {side.name}
                          </th>
                        ))}
                      </tr>
                    </thead>
                    <tbody className="divide-y divide-gray-100">
                      {comparison.topics.slice(0, 20).map((topic, row) => (
                        <tr key={row}>
                          {comparison.sides.map(side => {
                            const cell = topic.sides.find(s => s.investigationId === side.id);
                            return (
                              <td key={side.id} className="px-4 py-2 align-top">
                                {cell ? (
                                  <>
                                    <span className="text-gray-900">{cell.topic}</span>
                                    <span className="block text-xs text-gray-500">
                                      {[
                                        t('compare.claims', { count: cell.claims }),
                                        cell.contradicted > 0 && t('compare.contradicted', { count: cell.contradicted }),
                                        cell.fulfilled > 0 && t('compare.fulfilled', { count: cell.fulfilled }),
                                      ]
                                        .filter(Boolean)
                                        .join(' · ')}
                                    </span>
                                  </>
                                ) : (
                                  <span className="text-gray-400">—</span>
                                )}
                              </td>
                            );
                          })}
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              </Section>
            )}

            <Section title={t('compare.diverge')} help={t('compare.divergeHelp')}>
              {comparison.divergent.length === 0 ? (
                <Empty text={t('compare.noDivergent')} />
              ) : (
                <div className="grid divide-y divide-gray-100 sm:grid-cols-2 sm:divide-x sm:divide-y-0">
                  {comparison.sides.map(side => (
                    <div key={side.id} className="min-w-0">
                      <h3 className="px-4 pt-3 text-xs font-medium uppercase tracking-wide text-gray-500">{side.name}</h3>
                      <ul>
                        {comparison.divergent
                          .filter(p => p.investigationId === side.id)
                          .map(passage => (
                            <PassageRow key={passage.documentId} passage={passage} sideName={side.name} />
                          ))}
                      </ul>
                    </div>
                  ))}
                </div>
              )}
            </Section>

            <Section title={t('compare.timeline')} help={t('compare.timelineHelp')}>
              <Timeline comparison={comparison} />
            </Section>
          </>
        )}
      </div>
    </div>
  );
}
